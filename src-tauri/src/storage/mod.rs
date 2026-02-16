// Local persistence for UI state
// This module handles saving/loading pinned tabs, spaces, and other UI state

pub mod database;
pub mod crud;
pub mod spaces;
pub mod tabs;
pub mod folders;
pub mod state;
pub mod snippets;
pub mod history;

pub use database::{DatabaseManager, StorageError, StorageResult, get_default_db_path};
pub use spaces::{Space, CreateSpaceInput, UpdateSpaceInput};
pub use tabs::{Tab, TabType, CreateTabInput, UpdateTabInput};
pub use folders::{TabFolder, CreateFolderInput, UpdateFolderInput};
pub use snippets::{Snippet, CreateSnippetInput, UpdateSnippetInput};
pub use history::{ArchivedTab, ArchiveSearchResult};
pub use state::{AutoArchiveSettings, AppSettings};
