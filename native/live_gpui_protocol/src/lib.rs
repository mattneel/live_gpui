use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const QUEUE_CAPACITY: usize = 8;
pub const MAX_NODES: usize = 4096;
pub const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub props: BTreeMap<String, Value>,
    pub children: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tree {
    pub root: String,
    #[serde(deserialize_with = "unique_nodes")]
    pub nodes: BTreeMap<String, Node>,
}
fn unique_nodes<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, Node>, D::Error> {
    struct Nodes;
    impl<'de> serde::de::Visitor<'de> for Nodes {
        type Value = BTreeMap<String, Node>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a node map with unique keys")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut nodes = BTreeMap::new();
            while let Some((id, node)) = map.next_entry::<String, Node>()? {
                if nodes.len() >= MAX_NODES {
                    return Err(serde::de::Error::custom("tree node limit exceeded"));
                }
                if nodes.insert(id, node).is_some() {
                    return Err(serde::de::Error::custom("duplicate node ID"));
                }
            }
            Ok(nodes)
        }
    }
    deserializer.deserialize_map(Nodes)
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Snapshot {
        generation: u64,
        revision: u64,
        tree: Tree,
    },
    Patch {
        generation: u64,
        base_revision: u64,
        revision: u64,
        root: String,
        upsert: Vec<Node>,
        remove: Vec<String>,
    },
    Inspect {
        request: u64,
    },
    Shutdown,
}
fn valid_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 1024 {
        Err("node ID must contain 1..1024 bytes".into())
    } else {
        Ok(())
    }
}
fn validate_node(node: &Node) -> Result<(), String> {
    valid_id(&node.id)?;
    if !matches!(
        node.kind.as_str(),
        "column" | "row" | "list" | "text" | "button" | "input"
    ) {
        return Err("unsupported node kind".into());
    }
    if matches!(node.kind.as_str(), "text" | "button" | "input") && !node.children.is_empty() {
        return Err("leaf node has children".into());
    }
    if node.children.len() > MAX_NODES {
        return Err("too many children".into());
    }
    let mut children = BTreeSet::new();
    for id in &node.children {
        valid_id(id)?;
        if !children.insert(id) {
            return Err("duplicate child".into());
        }
    }
    for (key, value) in &node.props {
        let ok = match key.as_str() {
            "gap" => {
                matches!(node.kind.as_str(), "column" | "row" | "list")
                    && value.as_u64().is_some_and(|v| v <= 128)
            }
            "text" => matches!(node.kind.as_str(), "text" | "button") && value.is_string(),
            "value" | "placeholder" | "phx-change" => node.kind == "input" && value.is_string(),
            "phx-click" => node.kind == "button" && value.is_string(),
            "edit_revision" => node.kind == "input" && value.as_u64().is_some(),
            _ if key.starts_with("phx-value-") && key.len() > 10 => {
                matches!(node.kind.as_str(), "button" | "input")
                    && (value.is_null()
                        || value.is_boolean()
                        || value.is_number()
                        || value.is_string())
            }
            _ => false,
        };
        if !ok {
            return Err(format!("invalid property {key}"));
        }
    }
    Ok(())
}
pub fn validate_tree(tree: &Tree) -> Result<(), String> {
    valid_id(&tree.root)?;
    if tree.nodes.is_empty() || tree.nodes.len() > MAX_NODES {
        return Err("tree node limit exceeded".into());
    }
    for (id, node) in &tree.nodes {
        if id != &node.id {
            return Err("node map key differs from ID".into());
        }
        validate_node(node)?;
    }
    let mut seen = BTreeSet::new();
    let mut stack = vec![(&tree.root, 1usize)];
    while let Some((id, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            return Err("tree depth limit exceeded".into());
        }
        if !seen.insert(id) {
            return Err("cycle or multiple parents".into());
        }
        let node = tree.nodes.get(id).ok_or("missing child or root")?;
        for child in &node.children {
            stack.push((child, depth + 1));
        }
    }
    if seen.len() != tree.nodes.len() {
        return Err("unreachable nodes".into());
    }
    Ok(())
}
pub fn decode(bytes: &[u8]) -> Result<Command, String> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err("message exceeds 1 MiB".into());
    }
    let command: Command = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    match &command {
        Command::Snapshot {
            generation,
            revision,
            tree,
        } => {
            if *generation == 0 || *revision == 0 {
                return Err("generation and revision must be positive".into());
            }
            validate_tree(tree)?;
        }
        Command::Patch {
            generation,
            base_revision,
            revision,
            root,
            upsert,
            remove,
        } => {
            if *generation == 0 || base_revision.checked_add(1) != Some(*revision) {
                return Err("invalid patch revision".into());
            }
            valid_id(root)?;
            if upsert.len() > MAX_NODES || remove.len() > MAX_NODES {
                return Err("patch node limit exceeded".into());
            }
            let mut ids = BTreeSet::new();
            for node in upsert {
                validate_node(node)?;
                if !ids.insert(&node.id) {
                    return Err("duplicate upsert".into());
                }
            }
            let mut removed = BTreeSet::new();
            for id in remove {
                valid_id(id)?;
                if !removed.insert(id) || ids.contains(id) {
                    return Err("duplicate or conflicting removal".into());
                }
            }
        }
        _ => {}
    }
    Ok(command)
}
#[derive(Default)]
pub struct State {
    pub generation: u64,
    pub revision: u64,
    pub tree: Option<Tree>,
}
impl State {
    /// Returns false for a stale snapshot; errors leave the entire state unchanged.
    pub fn apply(&mut self, command: Command) -> Result<bool, String> {
        match command {
            Command::Snapshot {
                generation,
                revision,
                tree,
            } => {
                if generation == 0 || revision == 0 {
                    return Err("generation and revision must be positive".into());
                }
                if generation < self.generation
                    || (generation == self.generation && revision < self.revision)
                {
                    return Ok(false);
                }
                validate_tree(&tree)?;
                self.tree = Some(tree);
                self.generation = generation;
                self.revision = revision;
            }
            Command::Patch {
                generation,
                base_revision,
                revision,
                root,
                upsert,
                remove,
            } => {
                if generation == 0
                    || generation != self.generation
                    || base_revision != self.revision
                    || base_revision.checked_add(1) != Some(revision)
                {
                    return Err("generation or base revision mismatch".into());
                }
                valid_id(&root)?;
                if upsert.len() > MAX_NODES || remove.len() > MAX_NODES {
                    return Err("patch node limit exceeded".into());
                }
                let mut affected = BTreeSet::new();
                for node in &upsert {
                    validate_node(node)?;
                    if !affected.insert(node.id.clone()) {
                        return Err("duplicate upsert".into());
                    }
                }
                for id in &remove {
                    valid_id(id)?;
                    if !affected.insert(id.clone()) {
                        return Err("duplicate or conflicting removal".into());
                    }
                }
                let candidate = self.tree.as_mut().ok_or("patch before snapshot")?;
                // Move only affected originals into a rollback journal. No full tree copy.
                let originals: Vec<_> = affected
                    .into_iter()
                    .map(|id| {
                        let original = candidate.nodes.remove(&id);
                        (id, original)
                    })
                    .collect();
                let original_root = std::mem::replace(&mut candidate.root, root);
                for node in upsert {
                    candidate.nodes.insert(node.id.clone(), node);
                }
                if let Err(error) = validate_tree(candidate) {
                    candidate.root = original_root;
                    for (id, original) in originals {
                        candidate.nodes.remove(&id);
                        if let Some(node) = original {
                            candidate.nodes.insert(id, node);
                        }
                    }
                    return Err(error);
                }
                self.revision = revision;
            }
            _ => return Err("not a tree command".into()),
        }
        Ok(true)
    }
}

#[cfg(test)]
mod direct_boundary_tests {
    use super::*;
    #[test]
    fn direct_apply_rejects_zero_version_and_wrong_kind_property() {
        let node = Node {
            id: "root".into(),
            kind: "input".into(),
            props: BTreeMap::from([("gap".into(), Value::from(2))]),
            children: vec![],
        };
        let tree = Tree {
            root: "root".into(),
            nodes: BTreeMap::from([("root".into(), node)]),
        };
        let mut state = State::default();
        assert!(state
            .apply(Command::Snapshot {
                generation: 0,
                revision: 1,
                tree: tree.clone()
            })
            .is_err());
        assert!(state
            .apply(Command::Snapshot {
                generation: 1,
                revision: 0,
                tree: tree.clone()
            })
            .is_err());
        assert!(state
            .apply(Command::Snapshot {
                generation: 1,
                revision: 1,
                tree
            })
            .is_err());
        assert_eq!(state.generation, 0);
        assert!(state.tree.is_none());
    }
    #[test]
    fn rollback_restores_replaced_nodes_and_root() {
        let root = Node {
            id: "root".into(),
            kind: "column".into(),
            props: BTreeMap::new(),
            children: vec!["child".into()],
        };
        let child = Node {
            id: "child".into(),
            kind: "text".into(),
            props: BTreeMap::from([("text".into(), Value::from("original"))]),
            children: vec![],
        };
        let tree = Tree {
            root: "root".into(),
            nodes: BTreeMap::from([("root".into(), root), ("child".into(), child.clone())]),
        };
        let mut state = State::default();
        state
            .apply(Command::Snapshot {
                generation: 1,
                revision: 1,
                tree: tree.clone(),
            })
            .unwrap();
        let mut changed = child;
        changed.props.insert("text".into(), Value::from("changed"));
        assert!(state
            .apply(Command::Patch {
                generation: 1,
                base_revision: 1,
                revision: 2,
                root: "missing".into(),
                upsert: vec![changed],
                remove: vec![]
            })
            .is_err());
        assert_eq!(state.tree, Some(tree));
        assert_eq!(state.revision, 1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Command {
        decode(r#"{"type":"snapshot","generation":1,"revision":1,"tree":{"root":"a","nodes":{"a":{"id":"a","kind":"column","props":{},"children":["b"]},"b":{"id":"b","kind":"input","props":{"value":"😀"},"children":[]}}}}"#.as_bytes()).unwrap()
    }
    #[test]
    fn invalid_patch_is_atomic() {
        let mut state = State::default();
        state.apply(snapshot()).unwrap();
        let old = state.tree.clone();
        let command = decode(br#"{"type":"patch","generation":1,"base_revision":1,"revision":2,"root":"a","upsert":[],"remove":["b"]}"#).unwrap();
        assert!(state.apply(command).is_err());
        assert_eq!(state.revision, 1);
        assert_eq!(state.tree, old);
    }
    #[test]
    fn mismatched_generation_resyncs() {
        let mut state = State::default();
        state.apply(snapshot()).unwrap();
        assert!(state
            .apply(Command::Patch {
                generation: 2,
                base_revision: 1,
                revision: 2,
                root: "a".into(),
                upsert: vec![],
                remove: vec![]
            })
            .is_err());
        assert_eq!(state.generation, 1);
    }
    #[test]
    fn rejects_topology_and_payload_limits() {
        assert!(decode(&vec![b' '; MAX_MESSAGE_BYTES + 1]).is_err());
        let mut tree = match snapshot() {
            Command::Snapshot { tree, .. } => tree,
            _ => unreachable!(),
        };
        tree.nodes.get_mut("a").unwrap().children.push("b".into());
        assert!(validate_tree(&tree).is_err());
        tree.nodes.get_mut("a").unwrap().children = vec!["a".into()];
        assert!(validate_tree(&tree).is_err());
    }
    #[test]
    fn stale_snapshot_does_not_rewind() {
        let mut state = State::default();
        state.apply(snapshot()).unwrap();
        let mut command = snapshot();
        if let Command::Snapshot { generation, .. } = &mut command {
            *generation = 2;
        }
        state.apply(command).unwrap();
        assert!(!state.apply(snapshot()).unwrap());
        assert_eq!(state.generation, 2);
    }
    #[test]
    fn enforces_depth_and_reachability() {
        let mut nodes = BTreeMap::new();
        for i in 0..MAX_DEPTH {
            let id = i.to_string();
            nodes.insert(
                id.clone(),
                Node {
                    id,
                    kind: "column".into(),
                    props: BTreeMap::new(),
                    children: if i + 1 < MAX_DEPTH {
                        vec![(i + 1).to_string()]
                    } else {
                        vec![]
                    },
                },
            );
        }
        let mut tree = Tree {
            root: "0".into(),
            nodes,
        };
        assert!(validate_tree(&tree).is_ok());
        tree.nodes
            .get_mut(&(MAX_DEPTH - 1).to_string())
            .unwrap()
            .children
            .push("extra".into());
        tree.nodes.insert(
            "extra".into(),
            Node {
                id: "extra".into(),
                kind: "text".into(),
                props: BTreeMap::new(),
                children: vec![],
            },
        );
        assert!(validate_tree(&tree).is_err());
        tree.nodes
            .get_mut(&(MAX_DEPTH - 1).to_string())
            .unwrap()
            .children
            .clear();
        assert!(validate_tree(&tree).is_err());
    }
    #[test]
    fn duplicate_json_node_ids_are_not_silently_overwritten() {
        assert!(decode(br#"{"type":"snapshot","generation":1,"revision":1,"tree":{"root":"a","nodes":{"a":{"id":"a","kind":"text","props":{},"children":[]},"a":{"id":"a","kind":"text","props":{},"children":[]}}}}"#).is_err());
    }
}
