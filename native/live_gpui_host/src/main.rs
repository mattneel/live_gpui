mod node;
mod text;
use gpui::{prelude::*, *};
use live_gpui_protocol::{decode, Command, State, MAX_MESSAGE_BYTES, QUEUE_CAPACITY};
use node::*;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, Write},
    sync::{mpsc, LazyLock},
};

struct Output {
    sender: mpsc::SyncSender<Value>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
static OUTPUT: LazyLock<Output> = LazyLock::new(|| {
    let (sender, receiver) = mpsc::sync_channel::<Value>(64);
    let thread = std::thread::spawn(move || {
        let stdout = io::stdout();
        let mut writer = stdout.lock();
        for message in receiver {
            if serde_json::to_writer(&mut writer, &message).is_err()
                || writer.write_all(b"\n").is_err()
                || writer.flush().is_err()
                || message["type"] == "closed"
            {
                break;
            }
        }
    });
    Output {
        sender,
        thread: Mutex::new(Some(thread)),
    }
});
fn emit(message: Value) {
    if let Err(error) = OUTPUT.sender.try_send(message) {
        eprintln!("native event transport stopped: {error}");
        // Never block immediate input or silently lose a protocol dependency.
        std::process::exit(2);
    }
}
enum Incoming {
    Command(Command),
    Invalid(String),
    Eof,
}
struct Host {
    state: State,
    entities: BTreeMap<String, Entity<NativeNode>>,
}
impl Host {
    fn resync(&self, reason: String) {
        emit(
            json!({"type":"resync","generation":self.state.generation,"revision":self.state.revision,"reason":reason}),
        );
    }
    fn process(&mut self, incoming: Incoming, window: &mut Window, cx: &mut Context<Self>) {
        match incoming {
            Incoming::Command(Command::Shutdown) | Incoming::Eof => {
                window.remove_window();
                cx.quit();
            }
            Incoming::Invalid(reason) => self.resync(reason),
            Incoming::Command(Command::Inspect { request }) => self.inspect(request, window, cx),
            Incoming::Command(command) => {
                let previous_generation = self.state.generation;
                match self.state.apply(command) {
                    Ok(true) => {
                        self.reconcile(previous_generation != self.state.generation, cx);
                        emit(
                            json!({"type":"applied","generation":self.state.generation,"revision":self.state.revision}),
                        );
                        cx.notify();
                    }
                    Ok(false) => {}
                    Err(reason) => self.resync(reason),
                }
            }
        }
    }
    fn reconcile(&mut self, generation_changed: bool, cx: &mut Context<Self>) {
        let tree = self.state.tree.as_ref().unwrap();
        self.entities.retain(|id, entity| {
            tree.nodes
                .get(id)
                .is_some_and(|node| entity.read(cx).node.kind == node.kind)
        });
        for (id, node) in &tree.nodes {
            if !self.entities.contains_key(id) {
                let entity = cx.new(|cx| {
                    NativeNode::new(node.clone(), self.state.generation, self.state.revision, cx)
                });
                self.entities.insert(id.clone(), entity);
            }
        }
        for (id, node) in &tree.nodes {
            let current = self.entities[id].read(cx);
            let children_changed = current.children.len() != node.children.len()
                || current
                    .children
                    .iter()
                    .zip(&node.children)
                    .any(|(entity, id)| entity.entity_id() != self.entities[id].entity_id());
            let children = children_changed.then(|| {
                node.children
                    .iter()
                    .map(|id| self.entities[id].clone())
                    .collect()
            });
            self.entities[id].update(cx, |entity, cx| {
                let node_changed = entity.node != *node;
                if node_changed {
                    entity.node = node.clone();
                    entity.update_properties();
                }
                if let Some(children) = children {
                    entity.children = children;
                }
                entity.generation = self.state.generation;
                entity.application_revision = self.state.revision;
                if node.kind == "input" {
                    let acknowledgement = node
                        .props
                        .get("edit_revision")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    entity
                        .text
                        .reconcile(string(node, "value"), acknowledgement);
                    if generation_changed && acknowledgement < entity.text.revision {
                        entity.event("phx-change");
                    }
                }
                let display_changed = entity.refresh_display();
                if node_changed || children_changed || display_changed {
                    cx.notify();
                }
            });
        }
    }
    fn inspect(&self, request: u64, window: &Window, cx: &Context<Self>) {
        let nodes: serde_json::Map<String, Value> = self.entities.iter().map(|(id, entity)| {
            let node = entity.read(cx);
            let mut diagnostic = json!({"kind":node.node.kind,"entity":entity.entity_id().as_u64(),"focused":node.focus.is_focused(window),"children":node.node.children,"text":string(&node.node,"text")});
            if node.node.kind == "input" {
                let selection = text::utf16_range(&node.text.value, &node.text.selection);
                diagnostic["value"] = json!(node.text.value); diagnostic["edit_revision"] = json!(node.text.revision);
                diagnostic["selection"] = json!([selection.start, selection.end]);
                diagnostic["marked"] = node.text.marked.as_ref().map(|r| { let r = text::utf16_range(&node.text.value, r); json!([r.start, r.end]) }).unwrap_or(Value::Null);
            }
            (id.clone(), diagnostic)
        }).collect();
        emit(
            json!({"type":"inspection","request":request,"generation":self.state.generation,"revision":self.state.revision,"nodes":nodes,"root":self.state.tree.as_ref().map(|tree| &tree.root)}),
        );
    }
}
impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("live-root")
            .size_full()
            .p_4()
            .bg(rgb(0xe8edf3))
            .text_color(rgb(0x18202a))
            .text_size(px(18.))
            .overflow_y_scroll()
            .children(
                self.state
                    .tree
                    .as_ref()
                    .and_then(|tree| self.entities.get(&tree.root))
                    .cloned(),
            )
    }
}
/// Reads a bounded line without ever allocating an unbounded buffer for a bad peer.
fn read_input(sender: async_channel::Sender<Incoming>) {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    loop {
        let mut bytes = Vec::new();
        let mut oversized = false;
        let mut eof = false;
        loop {
            let buffer = match reader.fill_buf() {
                Ok(buffer) => buffer,
                Err(error) => {
                    let _ = sender.send_blocking(Incoming::Invalid(error.to_string()));
                    let _ = sender.send_blocking(Incoming::Eof);
                    return;
                }
            };
            if buffer.is_empty() {
                eof = true;
                break;
            }
            let end = buffer.iter().position(|b| *b == b'\n');
            let count = end.map_or(buffer.len(), |i| i + 1);
            if !oversized {
                if bytes.len() + count > MAX_MESSAGE_BYTES + 1 {
                    oversized = true;
                    bytes.clear();
                } else {
                    bytes.extend_from_slice(&buffer[..count]);
                }
            }
            reader.consume(count);
            if end.is_some() {
                break;
            }
        }
        if eof && bytes.is_empty() && !oversized {
            let _ = sender.send_blocking(Incoming::Eof);
            return;
        }
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
        }
        let incoming = if oversized {
            Incoming::Invalid("message exceeds 1 MiB".into())
        } else {
            match decode(&bytes) {
                Ok(command) => Incoming::Command(command),
                Err(error) => Incoming::Invalid(error),
            }
        };
        if sender.send_blocking(incoming).is_err() {
            return;
        }
        if eof {
            let _ = sender.send_blocking(Incoming::Eof);
            return;
        }
    }
}
fn main() {
    LazyLock::force(&OUTPUT);
    // Only plain serde values and Commands cross threads. GPUI entities stay on this main thread.
    let (input_tx, input_rx) = async_channel::bounded(QUEUE_CAPACITY);
    std::thread::spawn(move || read_input(input_tx));
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("backspace", Backspace, Some("LiveInput")),
            KeyBinding::new("delete", Delete, Some("LiveInput")),
            KeyBinding::new("left", Left, Some("LiveInput")),
            KeyBinding::new("right", Right, Some("LiveInput")),
            KeyBinding::new("shift-left", SelectLeft, Some("LiveInput")),
            KeyBinding::new("shift-right", SelectRight, Some("LiveInput")),
            KeyBinding::new("home", Home, Some("LiveInput")),
            KeyBinding::new("end", End, Some("LiveInput")),
            KeyBinding::new("ctrl-a", SelectAll, Some("LiveInput")),
            KeyBinding::new("ctrl-c", Copy, Some("LiveInput")),
            KeyBinding::new("ctrl-x", Cut, Some("LiveInput")),
            KeyBinding::new("ctrl-v", Paste, Some("LiveInput")),
            KeyBinding::new("cmd-a", SelectAll, Some("LiveInput")),
            KeyBinding::new("cmd-c", Copy, Some("LiveInput")),
            KeyBinding::new("cmd-x", Cut, Some("LiveInput")),
            KeyBinding::new("cmd-v", Paste, Some("LiveInput")),
        ]);
        cx.on_window_closed(|cx| cx.quit()).detach();
        let bounds = Bounds::centered(None, size(px(720.), px(540.)), cx);
        match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("live_gpui".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx: &mut Context<Host>| {
                    cx.spawn_in(window, async move |this, cx| loop {
                        let incoming = input_rx.recv().await.unwrap_or(Incoming::Eof);
                        let closed = matches!(
                            &incoming,
                            Incoming::Eof | Incoming::Command(Command::Shutdown)
                        );
                        if this
                            .update_in(cx, |host, window, cx| host.process(incoming, window, cx))
                            .is_err()
                            || closed
                        {
                            break;
                        }
                    })
                    .detach();
                    Host {
                        state: State::default(),
                        entities: BTreeMap::new(),
                    }
                })
            },
        ) {
            Ok(_) => {
                emit(json!({"type":"ready"}));
                cx.activate(true);
            }
            Err(error) => {
                eprintln!("cannot open GPUI window: {error}");
                emit(json!({"type":"closed","reason":error.to_string()}));
                cx.quit();
            }
        }
    });
    emit(json!({"type":"closed"}));
    if let Some(thread) = OUTPUT.thread.lock().take() {
        let _ = thread.join();
    }
}
