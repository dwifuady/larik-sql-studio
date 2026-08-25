// Local persistence for UI state
// This module handles saving/loading pinned tabs, spaces, and other UI state

pub mod crud;
pub mod database;
pub mod folders;
pub mod paths;
pub mod history;
pub mod notes;
pub mod schema_cache;
pub mod snippets;
pub mod spaces;
pub mod state;
pub mod tabs;
pub mod virtual_references;

pub use database::{get_default_db_path, DatabaseManager, StorageError, StorageResult};
pub use folders::{CreateFolderInput, TabFolder, UpdateFolderInput};
pub use history::{ArchiveSearchResult, ArchivedTab};
pub use notes::StickyNote;
pub use snippets::{CreateSnippetInput, Snippet, UpdateSnippetInput};
pub use spaces::{CreateSpaceInput, Space, UpdateSpaceInput};
pub use state::{AppSettings, AutoArchiveSettings};
pub use tabs::{CreateTabInput, Tab, TabType, UpdateTabInput};
pub use virtual_references::VirtualReference;
