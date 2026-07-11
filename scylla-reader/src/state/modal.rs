//! Modal state — add-book text inputs, jump-to-chapter list state, and command palette.

use crate::models::Chapter;
use crate::state::palette_action::PaletteAction;

#[derive(Debug, PartialEq)]
pub enum Modal {
    None,
    AddBook {
        inputs: Vec<String>,
        cursor: usize,
        scroll_offset: usize,
    },
    JumpChapter {
        chapters: Vec<Chapter>,
        query: String,
        filtered: Vec<Chapter>,
        cursor: usize,
        scroll_offset: usize,
        show_titles: bool,
    },
    SessionPicker {
        book_url: String,
        cursor: usize,
        scroll_offset: usize,
        input: Option<String>,
        editing_id: Option<i64>,
        pending_delete_url: Option<String>,
    },
    CommandPalette {
        query: String,
        filtered: Vec<PaletteAction>,
        selected: usize,
    },
    InstallPlugin {
        url: String,
        cursor: usize,
        scroll_offset: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modal_none_equality() {
        assert_eq!(Modal::None, Modal::None);
    }

    #[test]
    fn test_modal_add_book_construction_and_field_access() {
        let modal = Modal::AddBook {
            inputs: vec!["url1".into(), "url2".into()],
            cursor: 1,
            scroll_offset: 0,
        };
        if let Modal::AddBook {
            inputs,
            cursor,
            scroll_offset,
        } = &modal
        {
            assert_eq!(inputs.len(), 2);
            assert_eq!(inputs[0], "url1");
            assert_eq!(inputs[1], "url2");
            assert_eq!(*cursor, 1);
            assert_eq!(*scroll_offset, 0);
        } else {
            panic!("Expected AddBook variant");
        }
    }

    #[test]
    fn test_modal_jump_chapter_construction() {
        let chapters = vec![
            Chapter {
                title: "Ch1".into(),
                url: "url1".into(),
                order: 0,
            },
            Chapter {
                title: "Ch2".into(),
                url: "url2".into(),
                order: 1,
            },
        ];
        let modal = Modal::JumpChapter {
            chapters: chapters.clone(),
            query: "test".into(),
            filtered: chapters.clone(),
            cursor: 1,
            scroll_offset: 0,
            show_titles: true,
        };
        if let Modal::JumpChapter {
            chapters,
            query,
            filtered,
            cursor,
            scroll_offset,
            show_titles,
        } = &modal
        {
            assert_eq!(chapters.len(), 2);
            assert_eq!(query, "test");
            assert_eq!(filtered.len(), 2);
            assert_eq!(*cursor, 1);
            assert_eq!(*scroll_offset, 0);
            assert!(*show_titles);
        } else {
            panic!("Expected JumpChapter variant");
        }
    }

    #[test]
    fn test_modal_session_picker_construction_with_pending_delete() {
        let modal = Modal::SessionPicker {
            book_url: "http://example.com".into(),
            cursor: 0,
            scroll_offset: 0,
            input: Some("my session".into()),
            editing_id: Some(42),
            pending_delete_url: Some("http://example.com/delete".into()),
        };
        if let Modal::SessionPicker {
            book_url,
            cursor,
            scroll_offset,
            input,
            editing_id,
            pending_delete_url,
        } = &modal
        {
            assert_eq!(book_url, "http://example.com");
            assert_eq!(*cursor, 0);
            assert_eq!(*scroll_offset, 0);
            assert_eq!(input.as_deref(), Some("my session"));
            assert_eq!(*editing_id, Some(42));
            assert_eq!(
                pending_delete_url.as_deref(),
                Some("http://example.com/delete")
            );
        } else {
            panic!("Expected SessionPicker variant");
        }
    }

    #[test]
    fn test_modal_command_palette_construction() {
        let action = PaletteAction {
            category: "test",
            label: "Test Action",
            keys: "Ctrl+T",
            handler: |_, _| {},
        };
        let modal = Modal::CommandPalette {
            query: "test".into(),
            filtered: vec![action],
            selected: 0,
        };
        if let Modal::CommandPalette {
            query,
            filtered,
            selected,
        } = &modal
        {
            assert_eq!(query, "test");
            assert_eq!(filtered.len(), 1);
            assert_eq!(filtered[0].label, "Test Action");
            assert_eq!(*selected, 0);
        } else {
            panic!("Expected CommandPalette variant");
        }
    }

    #[test]
    fn test_modal_partial_eq_different_variants_not_equal() {
        assert_ne!(
            Modal::None,
            Modal::AddBook {
                inputs: vec![],
                cursor: 0,
                scroll_offset: 0,
            }
        );
    }

    #[test]
    fn test_modal_install_plugin_construction() {
        let modal = Modal::InstallPlugin {
            url: "https://github.com/owner/repo".into(),
            cursor: 5,
            scroll_offset: 0,
        };
        if let Modal::InstallPlugin {
            url,
            cursor,
            scroll_offset,
        } = &modal
        {
            assert_eq!(url, "https://github.com/owner/repo");
            assert_eq!(*cursor, 5);
            assert_eq!(*scroll_offset, 0);
        } else {
            panic!("Expected InstallPlugin variant");
        }
    }

    #[test]
    fn test_palette_action_construction() {
        let action = PaletteAction {
            category: "navigation",
            label: "Go Home",
            keys: "g",
            handler: |_, _| {},
        };
        assert_eq!(action.category, "navigation");
        assert_eq!(action.label, "Go Home");
        assert_eq!(action.keys, "g");
    }
}
