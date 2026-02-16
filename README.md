# Larik SQL Studio

![Larik SQL Studio](./public/larik-icon.png)

**Larik SQL Studio** is a modern, workspace-centric SQL IDE built for developers who want a fast, beautiful, and efficient database management experience.

Use workspaces to organize your database connections, and tabs to manage your queries. Features an Arc-style sidebar, command palette, and a focus on keyboard-driven workflows.

> [!NOTE] 
> **Built with AI Assistance**
> 
> This project is an experiment in AI-driven development. Approximately 90% of the code was generated with the assistance of LLMs. While we strive for high quality, some code patterns may reflect this origin. We welcome contributions and refactoring from the community!

## ✨ Features

- **Workspace-Centric**: Group related database connections into spaces.
- **Modern UI**: Dark mode, smooth animations, and a clean interface built with React & TailwindCSS.
- **Keyboard First**: Command palette (`Ctrl+Shift+P`) for quick actions.
- **Performance**: Powered by Rust (Tauri) for a lightweight footprint.
- **SQL Support**: 
    - MS SQL Server (via `tiberius`)
    - SQLite (via `rusqlite`)

## 🚀 Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) (v18+)
- [Rust](https://www.rust-lang.org/tools/install) (latest stable)

### Installation

1.  Clone the repository:
    ```bash
    git clone https://github.com/yourusername/larik-sql-studio.git
    cd larik-sql-studio
    ```

2.  Install dependencies:
    ```bash
    npm install
    ```

3.  Run in development mode:
    ```bash
    npm run tauri dev
    ```

## 🛠️ Tech Stack

- **Frontend**: React, TypeScript, Vite, TailwindCSS, Zustand
- **Backend**: Rust, Tauri v2, Rusqlite, Tiberius

## 🤝 Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
