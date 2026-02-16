import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

// Prevent default browser context menu globally, but allow for inputs/textareas
const handleContextMenu = (e: MouseEvent) => {
  const target = e.target as HTMLElement;
  const isInput = target.tagName === 'INPUT' ||
    target.tagName === 'TEXTAREA' ||
    target.isContentEditable ||
    target.closest('.monaco-editor'); // Allow Monaco to handle its own context menu

  if (!isInput) {
    e.preventDefault();
  }
};

window.addEventListener('contextmenu', handleContextMenu);

// Prevent global Ctrl+A (Select All) and DevTools shortcuts except in inputs/textareas
window.addEventListener('keydown', (e) => {
  // Ctrl+A / Cmd+A
  if ((e.ctrlKey || e.metaKey) && e.key === 'a') {
    const target = e.target as HTMLElement;
    const isInput = target.tagName === 'INPUT' ||
      target.tagName === 'TEXTAREA' ||
      target.isContentEditable ||
      target.closest('[data-allow-select-all]');

    if (!isInput) {
      e.preventDefault();
    }
  }

  // Prevent Ctrl+Shift+C and Ctrl+Shift+I from opening DevTools
  // (We use Ctrl+Shift+C for copy with headers in ResultsGrid)
  if ((e.ctrlKey || e.metaKey) && e.shiftKey && (e.key === 'C' || e.key === 'c' || e.key === 'I' || e.key === 'i')) {
    // We always prevent default for these to stop DevTools from opening.
    // Locally, we still catch these events in ResultsGrid because they bubble.
    e.preventDefault();
  }
}, false);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
