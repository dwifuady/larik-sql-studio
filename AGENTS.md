# Project Overview: Larik SQL Studio

This document provides an overview of the "Larik SQL Studio" project, serving as instructional context for future interactions with the AI Agent.

## Project Overview

**Larik SQL Studio** is a workspace-centric SQL studio application built with Tauri, combining a modern web frontend with a powerful Rust backend. Its primary purpose is to provide an integrated environment for SQL development and management.

**Key Technologies:**

*   **Application Framework:** Tauri (for cross-platform desktop application development using web technologies).
*   **Frontend:**
    *   **Framework:** React
    *   **Language:** TypeScript
    *   **Build Tool:** Vite
    *   **Styling:** TailwindCSS (v4)
    *   **State Management:** Zustand (using slice pattern)
    *   **UI Components:** Monaco Editor (for SQL editing), Lucide React (icons), Radix UI (via shadcn/ui patterns).
    *   **Testing:** Vitest
*   **Backend:**
    *   **Language:** Rust
    *   **Database Interactions:**
        *   `rusqlite`: For SQLite database operations.
        *   `tiberius`: For connecting to MS-SQL servers.
        *   `bb8` & `bb8-tiberius`: For MS-SQL connection pooling.
    *   **Serialization:** `serde` (for efficient data serialization/deserialization).
    *   **Utilities:** `uuid`, `thiserror`, `directories`, `tokio` (async runtime), `chrono` (date/time), `async-trait`.

## Building and Running

The project utilizes `npm` scripts to manage both the frontend (Vite) and backend (Tauri/Rust) build processes.

*   **Development Mode:**
    To run the application in development mode (with hot-reloading for the frontend and Rust compilation), use:
    ```bash
    npm run tauri dev
    ```

*   **Building for Production:**
    To build the application for production (creating an executable installer or application bundle), use:
    ```bash
    npm run tauri build
    ```

*   **Testing:**
    To run frontend tests:
    ```bash
    npm run test
    ```

*   **Frontend-only Development (if needed):**
    For frontend-only development and preview, you can use standard Vite commands, but these will not interact with the Rust backend:
    ```bash
    npm run dev      # Starts Vite development server
    npm run build    # Builds frontend assets
    npm run preview  # Previews built frontend assets
    ```

## Design Quality

This project uses [Impeccable](https://github.com/pbakaus/impeccable) to catch UI anti-patterns and design quality issues. Before every UI-related commit, run:

```bash
npx impeccable detect src/
```

This scans for overused fonts, low-contrast text, dated easing curves, and other "AI slop" patterns. Fix any findings before committing.

Key anti-patterns to avoid:
- **gray-on-color:** Gray text (`text-gray-*`) on any colored background (`bg-*`), including hover states
- **overused-font:** Inter, Geist, Plus Jakarta Sans, Space Grotesk — these are AI-overused fonts. Stick to the system font stack.
- **bounce-easing:** `cubic-bezier` values with bounce/elastic feel. Use `cubic-bezier(0.16, 1, 0.3, 1)` or `ease-out` instead.

PRODUCT.md and DESIGN.md at the project root define the design system. Read them before making UI changes.

## Development Conventions

*   **IDE Setup:**
    *   [VS Code](https://code.visualstudio.com/)
    *   [Tauri Extension](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode)
    *   [rust-analyzer Extension](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
*   **Project Structure:**
    *   **Frontend (`src/`):**
        *   `api/`: Frontend API layer for communicating with Tauri commands.
        *   `components/`: React components (UI building blocks).
        *   `hooks/`: Custom React hooks.
        *   `store/`: Zustand state management stores (split into slices).
        *   `types/`: TypeScript type definitions.
        *   `utils/`: Utility functions.
    *   **Backend (`src-tauri/src/`):**
        *   `commands.rs`: Tauri command definitions (callable from frontend).
        *   `db/`: Database connection and management logic.
        *   `export/`: Data export functionality.
        *   `storage/`: Local storage management (SQLite interactions).
        *   `capabilities/`: Tauri capabilities configuration.
*   **Code Style & Linting:**
    *   TypeScript for the frontend.
    *   Rust for the backend.
    *   `tsconfig.json` defines TypeScript compilation rules.
    *   Rust formatting (likely `rustfmt`) and linting (`clippy`) are implicitly handled by the Rust toolchain.
*   **State Management:** Frontend state is managed using **Zustand**, with logic split into "slices" (e.g., `queriesSlice`, `resultsSlice`, `tabsSlice`) in `src/store/slices`.
*   **Styling:** Frontend styling is handled by **TailwindCSS**.
