use ropey::Rope;

/// One replacement in a buffer, by char offset. Every edit carries the text
/// it removed, so it can be inverted exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub at: usize,
    pub removed: String,
    pub inserted: String,
}

impl Edit {
    pub fn apply(&self, rope: &mut Rope) {
        let end = self.at + self.removed.chars().count();
        debug_assert_eq!(rope.slice(self.at..end), self.removed.as_str());
        rope.remove(self.at..end);
        rope.insert(self.at, &self.inserted);
    }

    pub fn inverse(&self) -> Edit {
        Edit {
            at: self.at,
            removed: self.inserted.clone(),
            inserted: self.removed.clone(),
        }
    }

    pub fn is_insert(&self) -> bool {
        self.removed.is_empty()
    }

    /// Char offset just past the inserted text.
    pub fn end(&self) -> usize {
        self.at + self.inserted.chars().count()
    }
}
