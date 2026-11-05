export function mountFeedback(root: HTMLElement, options: {
  source: "site" | "app";
  version: string;
  commit?: string;
  context?: () => null;
}): () => void;
