// The prompt of the Excel guide's "use an outside AI" part (v1.1, A-OWNER-79): the text is the catalog's own (one per
// interface language) and the copy button hands exactly that text to the clipboard function it is given.
import { t } from '../i18n/index.ts';

/** The full prompt in the interface language, with its placeholders ([TOPIC] and so on) left for the user to fill in */
export function outsidePrompt(): string {
  return t('banks.guide.aiPrompt');
}

/** Copies the full prompt with `copy` (the clipboard helper); resolves to whether it worked */
export async function copyOutsidePrompt(copy: (text: string) => Promise<boolean>): Promise<boolean> {
  return copy(outsidePrompt());
}
