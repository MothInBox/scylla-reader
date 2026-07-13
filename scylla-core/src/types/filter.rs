#[derive(Debug, Clone, PartialEq)]
pub enum BackendKind {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LibraryFilter {
    All,
    ByStatus(crate::types::book::BookStatus),
    ByTag(String),
    Backend(String),
}
