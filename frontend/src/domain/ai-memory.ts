// AI Memory (v0.8): when a finished answer offers to save a note. Only an explicit request to remember something counts
// (a study question that merely contains the word "remember" must not), in English, Chinese and Japanese. Pure.

const PATTERNS: readonly RegExp[] = [
  // "Remember that …", "Please remember …"
  /^(please\s+)?remember\b/i,
  // 记住… / 请记住… / 帮我记住… / 请帮我记住…
  /^(请)?(帮我)?记住/,
  // 覚えておいて…
  /^覚えておいて/,
  // …を覚えておいて / …覚えておいてください (at the end, optionally with 。 or !)
  /覚えておいて(ください)?[。！!]?$/,
];

/** Whether the user's message asks the AI to remember something (then the page offers to save a note) */
export function wantsMemory(text: string): boolean {
  const s = text.trim();
  return !!s && PATTERNS.some((p) => p.test(s));
}

/** Characters as the AI service counts them (Unicode code points) */
export const memoryLength = (s: string): number => [...s].length;
