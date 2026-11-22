// Subsequence fuzzy scorer for the command palette. Higher is better; null = no match.
export function fuzzyScore(query: string, text: string): number | null {
  const q = query.toLowerCase().trim();
  if (!q) return 0;
  const t = text.toLowerCase();
  let score = 0;
  let ti = 0;
  let prev = -2;
  for (const ch of q) {
    const i = t.indexOf(ch, ti);
    if (i < 0) return null;
    score += 1;
    if (i === prev + 1) score += 3; // consecutive
    if (i === 0 || /[\s\-_/:.]/.test(t[i - 1] ?? "")) score += 2; // word start
    prev = i;
    ti = i + 1;
  }
  return score - t.length * 0.01; // prefer shorter labels on ties
}

export function rank<T>(items: T[], query: string, label: (t: T) => string): T[] {
  return items
    .map((item) => ({ item, s: fuzzyScore(query, label(item)) }))
    .filter((x): x is { item: T; s: number } => x.s !== null)
    .sort((a, b) => b.s - a.s)
    .map((x) => x.item);
}
