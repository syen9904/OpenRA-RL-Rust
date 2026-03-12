//! MiniYAML parser matching OpenRA's MiniYaml.cs.
//!
//! OpenRA uses a tab-indented YAML variant with:
//! - `Key: Value` pairs (colon-space separated)
//! - Tab or 4-space indentation for nesting
//! - `# comments`
//! - `Inherits: ^ParentName` for inheritance
//! - `-TraitName:` for trait/node removal
//! - `Trait@INSTANCE:` for named instances
//!
//! Reference: OpenRA.Game/MiniYaml.cs

use std::collections::BTreeMap;

/// A node in the MiniYAML tree.
#[derive(Debug, Clone)]
pub struct MiniYamlNode {
    pub key: String,
    pub value: String,
    pub children: Vec<MiniYamlNode>,
}

impl MiniYamlNode {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        MiniYamlNode {
            key: key.into(),
            value: value.into(),
            children: Vec::new(),
        }
    }

    /// Get a child node by key, or None if not found.
    pub fn child(&self, key: &str) -> Option<&MiniYamlNode> {
        self.children.iter().find(|c| c.key == key)
    }

    /// Get a child node's value by key, or None if not found.
    pub fn child_value(&self, key: &str) -> Option<&str> {
        self.child(key).map(|c| c.value.as_str())
    }
}

/// Parse a MiniYAML string into a list of top-level nodes.
pub fn parse(text: &str) -> Vec<MiniYamlNode> {
    let mut parsed: Vec<(usize, String, String)> = Vec::new();

    for line in text.lines() {
        // Determine indentation level
        let mut level = 0usize;
        let mut spaces = 0usize;
        let mut text_start = 0usize;

        for (i, ch) in line.char_indices() {
            match ch {
                '\t' => {
                    level += 1;
                    spaces = 0;
                    text_start = i + 1;
                }
                ' ' => {
                    spaces += 1;
                    if spaces >= 4 {
                        spaces = 0;
                        level += 1;
                    }
                    text_start = i + 1;
                }
                _ => {
                    text_start = i;
                    break;
                }
            }
        }

        let rest = &line[text_start..];

        // Skip empty lines and pure comment lines
        if rest.is_empty() {
            continue;
        }

        // Strip comments (# not preceded by \)
        let content = strip_comment(rest);
        let content = content.trim();
        if content.is_empty() {
            continue;
        }

        // Split into key: value
        let (key, value) = if let Some(colon_pos) = content.find(':') {
            let k = content[..colon_pos].trim().to_string();
            let v = content[colon_pos + 1..].trim().to_string();
            // Remove escape characters from \#
            let v = v.replace("\\#", "#");
            (k, v)
        } else {
            (content.trim().to_string(), String::new())
        };

        if key.is_empty() {
            continue;
        }

        parsed.push((level, key, value));
    }

    // Build tree from flat list using a stack
    build_tree(&parsed)
}

/// Strip inline comment from a line. The `#` character starts a comment
/// unless preceded by `\`.
fn strip_comment(s: &str) -> &str {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' && (i == 0 || bytes[i - 1] != b'\\') {
            return &s[..i];
        }
    }
    s
}

/// Build a tree of MiniYamlNodes from a flat list of (level, key, value).
fn build_tree(items: &[(usize, String, String)]) -> Vec<MiniYamlNode> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < items.len() {
        i = build_node(items, i, 0, &mut result);
    }
    result
}

/// Recursively build one node and its children. Returns next index to process.
fn build_node(
    items: &[(usize, String, String)],
    start: usize,
    expected_level: usize,
    siblings: &mut Vec<MiniYamlNode>,
) -> usize {
    if start >= items.len() || items[start].0 < expected_level {
        return start;
    }

    let (_, ref key, ref value) = items[start];
    let mut node = MiniYamlNode {
        key: key.clone(),
        value: value.clone(),
        children: Vec::new(),
    };

    let mut i = start + 1;
    // Collect children (items at level > expected_level)
    while i < items.len() && items[i].0 > expected_level {
        i = build_node(items, i, expected_level + 1, &mut node.children);
    }

    siblings.push(node);
    i
}

/// Parse multiple YAML files and merge them into a unified tree.
/// Later files override earlier ones (last-wins for matching keys).
pub fn parse_and_merge(sources: &[&str]) -> Vec<MiniYamlNode> {
    let mut merged = Vec::new();
    for source in sources {
        let nodes = parse(source);
        merged = merge_nodes(merged, nodes);
    }
    merged
}

/// Merge two node lists. Override nodes replace existing ones (by key),
/// new nodes are appended. Children are merged recursively.
fn merge_nodes(base: Vec<MiniYamlNode>, overrides: Vec<MiniYamlNode>) -> Vec<MiniYamlNode> {
    let mut result = base;
    for ov in overrides {
        if let Some(existing) = result.iter_mut().find(|n| n.key == ov.key) {
            // Merge value and children
            if !ov.value.is_empty() {
                existing.value = ov.value;
            }
            let merged_children = merge_nodes(
                std::mem::take(&mut existing.children),
                ov.children,
            );
            existing.children = merged_children;
        } else {
            result.push(ov);
        }
    }
    result
}

/// Resolve all `Inherits:` and `-Key:` directives in a merged tree.
///
/// `Inherits: ^ParentName` copies the parent's children into the current node.
/// `-TraitName:` removes a previously inherited or defined trait.
///
/// The `tree` is the top-level lookup table (actor name -> node) for resolving
/// parent references.
pub fn resolve_inherits(nodes: Vec<MiniYamlNode>) -> Vec<MiniYamlNode> {
    // Build a lookup table of top-level nodes for inheritance resolution.
    let lookup: BTreeMap<String, MiniYamlNode> = nodes.iter()
        .filter(|n| n.key.starts_with('^'))
        .map(|n| (n.key.clone(), n.clone()))
        .collect();

    nodes.into_iter()
        .filter(|n| !n.key.starts_with('^'))  // Remove abstract parents from output
        .map(|n| resolve_node(n, &lookup))
        .collect()
}

/// Resolve a single node: process Inherits and removals in its children.
fn resolve_node(
    mut node: MiniYamlNode,
    lookup: &BTreeMap<String, MiniYamlNode>,
) -> MiniYamlNode {
    let mut resolved: Vec<MiniYamlNode> = Vec::new();

    for child in node.children.drain(..) {
        if child.key == "Inherits" || child.key.starts_with("Inherits@") {
            // Resolve parent
            let parent_name = &child.value;
            if let Some(parent) = lookup.get(parent_name.as_str()) {
                // Recursively resolve the parent first
                let parent_resolved = resolve_node(parent.clone(), lookup);
                // Merge parent's children into our resolved list
                for pc in parent_resolved.children {
                    merge_into_resolved(&mut resolved, pc);
                }
            }
        } else if child.key.starts_with('-') {
            // Remove a previously defined node
            let to_remove = &child.key[1..];
            resolved.retain(|n| n.key != to_remove);
        } else {
            // Regular node: merge into resolved
            merge_into_resolved(&mut resolved, child);
        }
    }

    // Recursively resolve children of children
    node.children = resolved.into_iter()
        .map(|c| resolve_node(c, lookup))
        .collect();
    node
}

/// Merge a node into a resolved list: if a node with the same key exists,
/// merge their children. Otherwise append.
fn merge_into_resolved(resolved: &mut Vec<MiniYamlNode>, node: MiniYamlNode) {
    if let Some(existing) = resolved.iter_mut().find(|n| n.key == node.key) {
        // Merge value
        if !node.value.is_empty() {
            existing.value = node.value;
        }
        // Merge children
        let merged = merge_nodes(
            std::mem::take(&mut existing.children),
            node.children,
        );
        existing.children = merged;
    } else {
        resolved.push(node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_key_value() {
        let yaml = "Name: Test\nValue: 42\n";
        let nodes = parse(yaml);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].key, "Name");
        assert_eq!(nodes[0].value, "Test");
        assert_eq!(nodes[1].key, "Value");
        assert_eq!(nodes[1].value, "42");
    }

    #[test]
    fn parse_nested() {
        let yaml = "Parent:\n\tChild1: A\n\tChild2: B\n";
        let nodes = parse(yaml);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].key, "Parent");
        assert_eq!(nodes[0].children.len(), 2);
        assert_eq!(nodes[0].children[0].key, "Child1");
        assert_eq!(nodes[0].children[0].value, "A");
    }

    #[test]
    fn parse_spaces_indentation() {
        let yaml = "Parent:\n    Child: Value\n";
        let nodes = parse(yaml);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].children.len(), 1);
        assert_eq!(nodes[0].children[0].key, "Child");
    }

    #[test]
    fn parse_deep_nesting() {
        let yaml = "A:\n\tB:\n\t\tC: deep\n";
        let nodes = parse(yaml);
        assert_eq!(nodes[0].key, "A");
        assert_eq!(nodes[0].children[0].key, "B");
        assert_eq!(nodes[0].children[0].children[0].key, "C");
        assert_eq!(nodes[0].children[0].children[0].value, "deep");
    }

    #[test]
    fn parse_comments() {
        let yaml = "Key: Value # comment\n# full comment line\nOther: Data\n";
        let nodes = parse(yaml);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].key, "Key");
        assert_eq!(nodes[0].value, "Value");
        assert_eq!(nodes[1].key, "Other");
    }

    #[test]
    fn parse_named_instances() {
        let yaml = "Actor:\n\tArmament@PRIMARY:\n\t\tWeapon: Vulcan\n\tArmament@SECONDARY:\n\t\tWeapon: Missile\n";
        let nodes = parse(yaml);
        assert_eq!(nodes[0].children.len(), 2);
        assert_eq!(nodes[0].children[0].key, "Armament@PRIMARY");
        assert_eq!(nodes[0].children[1].key, "Armament@SECONDARY");
    }

    #[test]
    fn resolve_simple_inherits() {
        let yaml = "\
^Base:
\tHealth:
\t\tHP: 100
\tMobile:

Child:
\tInherits: ^Base
\tArmament:
";
        let nodes = parse(yaml);
        let resolved = resolve_inherits(nodes);
        // ^Base should be removed from output
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].key, "Child");
        // Should have Health, Mobile (inherited) + Armament (own)
        let keys: Vec<&str> = resolved[0].children.iter().map(|c| c.key.as_str()).collect();
        assert!(keys.contains(&"Health"));
        assert!(keys.contains(&"Mobile"));
        assert!(keys.contains(&"Armament"));
    }

    #[test]
    fn resolve_inherits_with_removal() {
        let yaml = "\
^Base:
\tHealth:
\tMobile:
\tArmament:

Child:
\tInherits: ^Base
\t-Armament:
";
        let nodes = parse(yaml);
        let resolved = resolve_inherits(nodes);
        let keys: Vec<&str> = resolved[0].children.iter().map(|c| c.key.as_str()).collect();
        assert!(keys.contains(&"Health"));
        assert!(keys.contains(&"Mobile"));
        assert!(!keys.contains(&"Armament"));
    }

    #[test]
    fn resolve_inherits_override_value() {
        let yaml = "\
^Base:
\tHealth:
\t\tHP: 100

Child:
\tInherits: ^Base
\tHealth:
\t\tHP: 200
";
        let nodes = parse(yaml);
        let resolved = resolve_inherits(nodes);
        let health = resolved[0].child("Health").unwrap();
        assert_eq!(health.child_value("HP"), Some("200"));
    }

    #[test]
    fn merge_multiple_sources() {
        let base = "Actor:\n\tHP: 100\n";
        let overlay = "Actor:\n\tHP: 200\n\tSpeed: 5\n";
        let merged = parse_and_merge(&[base, overlay]);
        assert_eq!(merged[0].child_value("HP"), Some("200"));
        assert_eq!(merged[0].child_value("Speed"), Some("5"));
    }

    #[test]
    fn parse_empty_value() {
        let yaml = "TraitName:\n\tChild: value\n";
        let nodes = parse(yaml);
        assert_eq!(nodes[0].key, "TraitName");
        assert_eq!(nodes[0].value, "");
        assert_eq!(nodes[0].children.len(), 1);
    }

    #[test]
    fn parse_colon_in_value() {
        let yaml = "Key: host:port\n";
        let nodes = parse(yaml);
        // Only first colon splits
        assert_eq!(nodes[0].key, "Key");
        assert_eq!(nodes[0].value, "host:port");
    }
}
