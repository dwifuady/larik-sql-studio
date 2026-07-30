import { useState } from 'react';
import Editor, { type OnMount } from '@monaco-editor/react';
import { useAppStore } from '../store';

interface MonacoPreviewProps {
  content: string;
  language: 'json' | 'xml' | 'plaintext' | 'sql';
}

export function MonacoPreview({ content, language }: MonacoPreviewProps) {
  const theme = useAppStore((state) => state.theme);
  const [isLoading, setIsLoading] = useState(true);
  // Monaco measures its container when the editor is created. When the preview
  // panel mounts during a tab or result-tab switch that measurement lands
  // before the surrounding layout has settled, so the editor collapses to 5x5
  // and paints nothing — the panel just looks blank. One synchronous layout()
  // here re-measures against the settled container; automaticLayout below then
  // keeps it right while the panel is dragged wider.
  const handleMount: OnMount = (editor) => {
    setIsLoading(false);
    editor.layout();
  };

  // Resolve theme to Monaco theme
  const resolvedTheme = theme === 'system'
    ? window.matchMedia('(prefers-color-scheme: dark)').matches ? 'vs-dark' : 'vs'
    : theme === 'dark' ? 'vs-dark' : 'vs';

  return (
    <div className="relative w-full h-full">
      {isLoading && (
        <div className="absolute inset-0 flex items-center justify-center bg-[var(--bg-primary)] z-10">
          <div className="flex flex-col items-center gap-2">
            <div className="w-6 h-6 border-2 border-[var(--accent-color)] border-t-transparent rounded-full animate-spin" />
            <span className="text-xs text-[var(--text-secondary)]">Loading editor...</span>
          </div>
        </div>
      )}
      <Editor
        height="100%"
        language={language}
        theme={resolvedTheme}
        value={content}
        options={{
          readOnly: true,
          automaticLayout: true,
          minimap: { enabled: false },
          wordWrap: 'on',
          scrollBeyondLastLine: false,
          lineNumbers: 'on',
          renderLineHighlight: 'none',
          folding: true,
          fontSize: 13,
          padding: { top: 8, bottom: 8 },
          scrollbar: {
            vertical: 'visible',
            horizontal: 'visible',
            useShadows: false,
            verticalScrollbarSize: 10,
            horizontalScrollbarSize: 10
          }
        }}
        onMount={handleMount}
      />
    </div>
  );
}
