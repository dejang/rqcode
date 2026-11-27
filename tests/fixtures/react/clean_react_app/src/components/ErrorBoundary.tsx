import React from "react";

interface BoundaryProps {
  fallbackMessage: string;
  children?: React.ReactNode;
}

interface BoundaryState {
  hasError: boolean;
}

export class ErrorBoundary extends React.Component<BoundaryProps, BoundaryState> {
  public state: BoundaryState = { hasError: false };

  public componentDidCatch(_error: Error): void {
    this.setState({ hasError: true });
  }

  public render(): React.ReactNode {
    if (this.state.hasError) {
      return <p role="alert">{this.props.fallbackMessage}</p>;
    }
    return this.props.children;
  }
}
