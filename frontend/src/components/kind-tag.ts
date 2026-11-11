// Exam / Practice (v0.6.5): the name of a record's kind and the small tag that shows it (study records list and detail,
// the latest record on the home page). Exams use the quiet neutral tag, practice the soft blue one; the word carries the
// meaning, the tone only helps to tell the rows apart at a glance.
import { t } from '../i18n/index.ts';
import type { AttemptKind } from '../types/api.ts';
import { h } from '../utils/dom.ts';

export function kindName(kind: AttemptKind): string {
  return t(kind === 'practice' ? 'records.kind.practice' : 'records.kind.exam');
}

export function kindTag(kind: AttemptKind): HTMLElement {
  return h('span', { class: 'tag kind-tag ' + (kind === 'practice' ? 'b' : 'k') }, kindName(kind));
}
