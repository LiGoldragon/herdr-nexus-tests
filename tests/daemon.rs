use super::*;
use core_meta_herdr_nexus::CoreSet;
use emission_herdr_nexus::{ExternalOutcome, Herdr, Verb};
use herdr_nexus_core::generated::memory::PendingCreation;
use herdr_nexus_core::{Core, ExternalResult, InMemory, Memory};
use meta_signal_herdr_nexus::{ByteLimit, ConfigurationRefusal, Timeout};
use shared_translator::{MetaflowName, PeerBinding, PeerId, PeerIdentity, Place, ProcessId, Runtime, State as PlaceState, UserId};
use signal_herdr_nexus::{CreationId, InstanceId, Outcome, Pane, PaneId, Root, Sequence, Session, StorePhase, Workspace};

fn unique_place(name: &str) -> (std::path::PathBuf, Place) {
    let directory = std::env::temp_dir().join(format!(
        "herdr-nexus-external-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).expect("fixture directory");
    let text = directory.display().to_string();
    (directory, Place {
        metaflow_name: MetaflowName(name.into()),
        runtime: Runtime(text.clone()),
        state: PlaceState(text),
    })
}

fn current_identity() -> PeerIdentity {
    PeerIdentity {
        user_id: UserId(i64::from(nix::unistd::getuid().as_raw())),
        process_id: ProcessId(i64::from(std::process::id())),
    }
}

fn configure_until_final(bootstrap: &Bootstrap) {
    for setting in [
        CoreSet::StartTimeout(Timeout(1_000)),
        CoreSet::Polling(Timeout(1_000)),
        CoreSet::FrameCapacity(ByteLimit(65_536)),
        CoreSet::TransportTimeout(Timeout(1_000)),
    ] {
        assert!(matches!(bootstrap.configure(setting), core_meta_herdr_nexus::Response::Configured(_)));
    }
}

#[test]
fn accepted_public_connection_always_has_ready_snapshot() {
    let (directory, place) = unique_place("ready");
    let bootstrap = Arc::new(Bootstrap::bind(
        &place,
        herdr_nexus_wire::ValidAdmission {
            deadline: Duration::from_secs(1),
            frame_capacity: 65_536,
            retry_delay: Duration::from_millis(1),
        },
        current_identity(),
    ).expect("bootstrap"));
    configure_until_final(&bootstrap);
    let accepting = {
        let bootstrap = Arc::clone(&bootstrap);
        std::thread::spawn(move || {
            let _accepted = bootstrap.accept_public().expect("accepted public caller");
            bootstrap.snapshot().is_some()
        })
    };
    let connecting = {
        let path = bootstrap.paths().public.clone();
        std::thread::spawn(move || loop {
            match std::os::unix::net::UnixStream::connect(&path) {
                Ok(stream) => return stream,
                Err(_) => std::thread::yield_now(),
            }
        })
    };
    assert!(matches!(
        bootstrap.configure(CoreSet::SubscriberCapacity(8)),
        core_meta_herdr_nexus::Response::Configured(_)
    ));
    assert!(accepting.join().expect("accept thread"));
    let stream = connecting.join().expect("connect thread");
    drop(stream);
    drop(bootstrap);
    std::fs::remove_dir(directory).expect("fixture cleanup");
}

#[test]
fn frame_capacity_refusal_preserves_the_prior_setting() {
    let store = ConfigurationStore::default();
    assert!(matches!(
        store.configure(CoreSet::FrameCapacity(ByteLimit(herdr_nexus_wire::frame::HEADER_BYTES as i64)), || Ok(())),
        core_meta_herdr_nexus::Response::Configured(_)
    ));
    let before = store.partial.read().expect("partial").frame_capacity_bytes;
    assert!(matches!(
        store.configure(CoreSet::FrameCapacity(ByteLimit(herdr_nexus_wire::frame::HEADER_BYTES as i64 - 1)), || Ok(())),
        core_meta_herdr_nexus::Response::ConfigureRefused(ConfigurationRefusal::Invalid(_))
    ));
    assert_eq!(store.partial.read().expect("partial").frame_capacity_bytes, before);
}

fn creation_id(sequence: i64) -> CreationId {
    CreationId { instance_id: InstanceId("external".into()), sequence: Sequence(sequence) }
}

fn workspace(root: &str) -> Workspace {
    Workspace { session: Session("external".into()), root: Root(root.into()), variable_vector: Vec::new() }
}

#[test]
fn duplicate_created_pane_preserves_owner_and_pending_fact() {
    let pane = Pane { session: Session("external".into()), pane_id: PaneId("same".into()) };
    let mut core = Core::new(InMemory::default());
    core.prepare_creation(PendingCreation { creation_id: creation_id(1), principal: shared_translator::Principal::Login(UserId(1)), workspace: workspace("/first") }).unwrap();
    assert_eq!(core.create(creation_id(1), ExternalResult::Succeeded(pane.clone())), Outcome::Created(pane.clone()));
    core.prepare_creation(PendingCreation { creation_id: creation_id(2), principal: shared_translator::Principal::Login(UserId(2)), workspace: workspace("/second") }).unwrap();
    assert!(matches!(core.create(creation_id(2), ExternalResult::Succeeded(pane.clone())), Outcome::StoreRefused(ref refusal) if matches!(refusal.store_phase, StorePhase::Observed(_))));
    assert_eq!(core.memory().owned(&pane).unwrap().unwrap().workspace.root, Root("/first".into()));
    assert!(core.memory().pending(&creation_id(2)).unwrap().is_some());
}

#[test]
fn delayed_prompt_grant_is_refused_after_remove_and_rebind() {
    let store = ConfigurationStore::default();
    let caller = herdr_nexus_wire::frame::peer_caller(&std::os::unix::net::UnixStream::pair().unwrap().0).unwrap();
    let identity = PeerIdentity { user_id: caller.user_id.clone(), process_id: caller.process_id.clone() };
    let first = PeerBinding { peer_id: PeerId("first".into()), peer_identity: identity.clone() };
    assert!(matches!(store.configure(CoreSet::Peer(first.clone()), || Ok(())), core_meta_herdr_nexus::Response::Configured(_)));
    let (_, generation) = store.peer_for(&caller).expect("accepted grant");
    assert!(store.peer_grant_is_current(&caller, &first.peer_id, generation));
    let second = PeerBinding { peer_id: PeerId("second".into()), peer_identity: identity };
    assert!(matches!(store.configure(CoreSet::Peer(second), || Ok(())), core_meta_herdr_nexus::Response::ConfigureRefused(ConfigurationRefusal::Invalid(_))));
    assert!(matches!(store.configure(CoreSet::RemovePeer(first.peer_id.clone()), || Ok(())), core_meta_herdr_nexus::Response::Configured(_)));
    assert!(!store.peer_grant_is_current(&caller, &first.peer_id, generation));
    assert!(matches!(store.configure(CoreSet::Peer(first.clone()), || Ok(())), core_meta_herdr_nexus::Response::Configured(_)));
    let (_, rebound_generation) = store.peer_for(&caller).expect("rebound grant");
    assert_ne!(generation, rebound_generation);
    assert!(!store.peer_grant_is_current(&caller, &first.peer_id, generation));
    assert!(store.peer_grant_is_current(&caller, &first.peer_id, rebound_generation));
}

struct StaleListEmitter {
    pane: Pane,
    list_started: std::sync::mpsc::Sender<()>,
    release_list: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

fn pane_listing(pane: &Pane) -> emission_herdr_nexus::PaneListing {
    emission_herdr_nexus::PaneListing { pane: pane.clone(), terminal_id: emission_herdr_nexus::TerminalId("terminal".into()), workspace_id: emission_herdr_nexus::WorkspaceId("workspace".into()), tab_id: emission_herdr_nexus::TabId("tab".into()) }
}

fn agent_listing(pane: &Pane, state: signal_herdr_nexus::State) -> emission_herdr_nexus::AgentListing {
    emission_herdr_nexus::AgentListing { pane: pane.clone(), terminal_id: emission_herdr_nexus::TerminalId("terminal".into()), workspace_id: emission_herdr_nexus::WorkspaceId("workspace".into()), tab_id: emission_herdr_nexus::TabId("tab".into()), state }
}

impl ExternalEmitter for StaleListEmitter {
    fn emit(&self, _configuration: &ReadyCoreConfiguration, herdr: Herdr) -> EmissionResult {
        EmissionResult::External(match herdr.verb {
            Verb::WorkspaceCreate(_) => ExternalOutcome::Created(self.pane.clone()),
            Verb::AgentStart(_) => ExternalOutcome::Started(signal_herdr_nexus::State::Busy),
            Verb::AgentPrompt(_) => ExternalOutcome::Uncertain,
            Verb::PaneList => {
                self.list_started.send(()).unwrap();
                self.release_list.lock().unwrap().recv().unwrap();
                ExternalOutcome::Panes(vec![pane_listing(&self.pane)])
            }
            Verb::AgentList => ExternalOutcome::Agents(vec![agent_listing(&self.pane, signal_herdr_nexus::State::Open)]),
            _ => ExternalOutcome::Done,
        })
    }
}

fn test_configuration() -> Arc<ReadyCoreConfiguration> {
    Arc::new(ReadyCoreConfiguration { polling: Duration::from_millis(10), start_timeout: Duration::from_millis(20), frame_capacity_bytes: 4096, transport_timeout: Duration::from_millis(30), subscriber_capacity: 8 })
}

#[test]
fn stale_list_cannot_overwrite_uncertain_prompt_state() {
    use signal_herdr_nexus::{AgentKind, AgentName, Attempt, Command, Executable, Label, Program, Prompt_Data, Query, Start_Data, Text};
    let pane = Pane { session: Session("attempt-race".into()), pane_id: PaneId("pane".into()) };
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let emitter = Arc::new(StaleListEmitter { pane: pane.clone(), list_started: started_tx, release_list: std::sync::Mutex::new(release_rx) });
    let (directory, place) = unique_place("stale");
    let paths = SocketPaths::from(&place);
    let dispatcher = Arc::new(Dispatcher::with_emitter(&paths, emitter));
    let configuration = test_configuration();
    assert!(matches!(dispatcher.dispatch(Query::Create(workspace("/tmp")), Arc::clone(&configuration)), signal_herdr_nexus::Response::Created(Outcome::Created(_))));
    assert!(matches!(dispatcher.dispatch(Query::Start(Start_Data { pane: pane.clone(), command: Command { agent_name: AgentName("probe".into()), agent_kind: AgentKind::Codex, executable: Executable::Explicit(Program("/bin/true".into())), string_vector: Vec::new() }, label: Label("flow".into()) }), Arc::clone(&configuration)), signal_herdr_nexus::Response::Started(Outcome::Done)));
    let listing = { let dispatcher=Arc::clone(&dispatcher); let configuration=Arc::clone(&configuration); let session=pane.session.clone(); std::thread::spawn(move || dispatcher.dispatch(Query::List(session), configuration)) };
    started_rx.recv().unwrap();
    assert!(matches!(dispatcher.dispatch(Query::Prompt(Prompt_Data { pane: pane.clone(), text: Text("hello".into()) }), configuration), signal_herdr_nexus::Response::Prompted(Outcome::Uncertain)));
    release_tx.send(()).unwrap();
    let signal_herdr_nexus::Response::Listed(Outcome::Listed(known)) = listing.join().unwrap() else { panic!("complete list") };
    assert_eq!(known[0].state, signal_herdr_nexus::State::Busy);
    assert_eq!(known[0].attempt, Attempt::Uncertain);
    drop(dispatcher);
    std::fs::remove_dir(directory).unwrap();
}
