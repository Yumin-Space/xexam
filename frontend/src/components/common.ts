import type { Mode, Question } from '../types/api.ts';
import { h } from '../utils/dom.ts';

export const MODE_NAME: Record<string, string> = { all: '全部题目', weak: '错题本', retry: '重做错题' };

export function modeName(m: Mode | string): string {
  return MODE_NAME[m] ?? '';
}

export const kindLabel = (q: Pick<Question, 'kind'>): string => (q.kind === 'hotspot' ? '热点题' : q.kind === 'multi' ? '多选' : '单选');

export function brand(): HTMLElement {
  return h('div', { class: 'brand' },
    h('span', { class: 'logo', 'aria-hidden': 'true' }, h('i', { class: 'on' }), h('i'), h('i', { class: 'on' }), h('i')),
    h('h1', null, 'Xexam'));
}
