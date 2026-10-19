use std::collections::BTreeMap;

/// Objects keyed by name, from the first segment of each path.
pub(super) struct ObjectTree<T> {
    pub(super) roots: BTreeMap<String, ObjectNode<T>>,
}

impl<T> Default for ObjectTree<T> {
    fn default() -> Self {
        Self {
            roots: BTreeMap::new(),
        }
    }
}

impl<T> ObjectTree<T> {
    /// Stores `binding` at the dotted `path`.
    pub(super) fn insert(&mut self, path: &str, binding: T) {
        self.insert_segments(path.split('.'), binding);
    }

    /// Stores `binding` at the path of `segments`. An empty path stores nothing.
    pub(super) fn insert_segments<'a>(
        &mut self,
        segments: impl IntoIterator<Item = &'a str>,
        binding: T,
    ) {
        let mut segments = segments.into_iter();
        let Some(root) = segments.next() else {
            return;
        };

        self.roots
            .entry(root.to_owned())
            .or_default()
            .insert(segments, binding);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }
}

/// A name of an object tree, with the child names below it and its own binding.
pub(super) struct ObjectNode<T> {
    pub(super) children: BTreeMap<String, ObjectNode<T>>,
    pub(super) binding: Option<T>,
}

impl<T> Default for ObjectNode<T> {
    fn default() -> Self {
        Self {
            children: BTreeMap::new(),
            binding: None,
        }
    }
}

impl<T> ObjectNode<T> {
    fn insert<'a>(&mut self, mut segments: impl Iterator<Item = &'a str>, binding: T) {
        match segments.next() {
            None => self.binding = Some(binding),
            Some(head) => self
                .children
                .entry(head.to_owned())
                .or_default()
                .insert(segments, binding),
        }
    }
}
