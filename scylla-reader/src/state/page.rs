//! Page enum — defines which screen the app is showing.

#[derive(Debug, PartialEq, Clone)]
pub enum Page {
    Library,
    Settings,
    AddingBook,
    BookChapterJump,
    Reader,
    Jobs,
    InstallingPlugin,
}
