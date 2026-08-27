import React from 'react';

interface Props {
  children: React.ReactNode;
  fallback?: React.ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends React.Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error('[ErrorBoundary]', error, info);
  }

  render() {
    if (this.state.hasError) {
      return (
        this.props.fallback ?? (
          <div className="flex flex-col items-center justify-center p-6 text-sm text-[var(--text-muted)]">
            <p className="font-medium text-[var(--text-primary)]">Something went wrong</p>
            <p className="mt-1 max-w-[40ch] text-center text-xs opacity-70">
              {this.state.error?.message ?? 'Unknown error'}
            </p>
            <button
              className="mt-3 rounded border border-[var(--border-color)] px-3 py-1.5 text-xs hover:bg-[var(--bg-hover)]"
              onClick={() => this.setState({ hasError: false, error: null })}
            >
              Try again
            </button>
          </div>
        )
      );
    }
    return this.props.children;
  }
}
