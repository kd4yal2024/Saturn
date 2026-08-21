export type ResponsiveLayoutMode = 'desktop' | 'phone';

export type ViewportCapabilities = Readonly<{
  width: number;
  height: number;
  coarsePointer: boolean;
}>;

/**
 * Select the initial operating layout without treating a landscape phone as a
 * miniature desktop. A saved operator choice remains authoritative.
 */
export function preferredResponsiveLayout(
  saved: string | null | undefined,
  viewport: ViewportCapabilities,
): ResponsiveLayoutMode {
  if (saved === 'phone' || saved === 'desktop') return saved;

  const width = Math.max(0, Number(viewport.width) || 0);
  const height = Math.max(0, Number(viewport.height) || 0);
  if (width < 768) return 'phone';
  if (viewport.coarsePointer && height < 600 && width < 1024) return 'phone';
  return 'desktop';
}
