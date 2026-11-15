// Provider marks for the thinking-strength row of the AI settings (v1.6): inline SVG built with createElementNS, one
// colour (fill = currentColor, so light / dark follow the theme), hidden from screen readers (the button carries the name).
// Anthropic, Google and the Responses API family (the DeepSeek whale) use the Simple Icons paths; OpenAI uses the owner's traced mark, Gemma a star drawn here.
import type { AiEffortFamily } from '../types/api.ts';

export type EffortFamily = AiEffortFamily;
const SVGNS = 'http://www.w3.org/2000/svg';

/** The family buttons of the row, in order: [Default] then these */
export const EFFORT_FAMILIES: readonly EffortFamily[] = ['anthropic', 'openai', 'google', 'responses', 'gemma'];

/** The provider names are proper nouns and stay the same in every language */
export const PROVIDER_NAME: Record<EffortFamily, string> = { anthropic: 'Anthropic', openai: 'OpenAI', google: 'Google', responses: 'Responses API', gemma: 'Gemma' };

/** The request parameter each family button sends: the visible tooltip (not the vendor name), never translated */
export const EFFORT_PARAM: Record<EffortFamily, string> = { anthropic: 'output_config.effort', openai: 'reasoning_effort', google: 'thinking_level', responses: 'reasoning.effort', gemma: 'thinking_level' };

/** Tooltip of a family button: the parameter it sends */
export const effortTip = (f: EffortFamily): string => EFFORT_PARAM[f];

/** Accessible name of a family button: the provider and the parameter, e.g. "Anthropic, output_config.effort" */
export const effortAria = (f: EffortFamily): string => `${PROVIDER_NAME[f]}, ${EFFORT_PARAM[f]}`;

// Simple Icons 16.38.0 (CC0): the anthropic, google and deepseek paths, viewBox 0 0 24 24
export const ANTHROPIC_PATH = 'M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z';
export const GOOGLE_PATH = 'M12.48 10.92v3.28h7.84c-.24 1.84-.853 3.187-1.787 4.133-1.147 1.147-2.933 2.4-6.053 2.4-4.827 0-8.6-3.893-8.6-8.72s3.773-8.72 8.6-8.72c2.6 0 4.507 1.027 5.907 2.347l2.307-2.307C18.747 1.44 16.133 0 12.48 0 5.867 0 .307 5.387.307 12s5.56 12 12.173 12c3.573 0 6.267-1.173 8.373-3.36 2.16-2.16 2.84-5.213 2.84-7.667 0-.76-.053-1.467-.173-2.053H12.48z';
export const DEEPSEEK_PATH = 'M23.748 4.651c-.254-.124-.364.113-.512.233-.051.04-.094.09-.137.137-.372.397-.806.657-1.373.626-.829-.046-1.537.214-2.163.848-.133-.782-.575-1.248-1.247-1.548-.352-.155-.708-.311-.955-.65-.172-.24-.219-.509-.305-.774-.055-.16-.11-.323-.293-.35-.2-.031-.278.136-.356.276-.313.572-.434 1.202-.422 1.84.027 1.436.633 2.58 1.838 3.393.137.094.172.187.129.323-.082.28-.18.553-.266.833-.055.179-.137.218-.328.14a5.5 5.5 0 0 1-1.737-1.179c-.857-.828-1.631-1.743-2.597-2.46a12 12 0 0 0-.689-.47c-.985-.957.13-1.743.387-1.836.27-.098.094-.433-.778-.428-.872.003-1.67.295-2.687.685a3 3 0 0 1-.465.136 9.6 9.6 0 0 0-2.883-.101c-1.885.21-3.39 1.1-4.497 2.622C.082 8.776-.231 10.854.152 13.02c.403 2.284 1.568 4.175 3.36 5.653 1.857 1.533 3.997 2.284 6.438 2.14 1.482-.085 3.132-.284 4.994-1.86.47.234.962.328 1.78.398.629.058 1.235-.031 1.705-.129.735-.155.684-.836.418-.961-2.155-1.004-1.682-.595-2.112-.926 1.095-1.295 2.768-3.598 3.284-6.733.05-.346.115-.834.108-1.114-.004-.171.035-.238.23-.257a4.2 4.2 0 0 0 1.545-.475c1.397-.763 1.96-2.016 2.093-3.517.02-.23-.004-.467-.247-.588M11.58 18.168c-2.088-1.642-3.101-2.183-3.52-2.16-.39.024-.32.472-.234.763.09.288.207.487.371.74.114.167.192.416-.113.603-.673.416-1.842-.14-1.897-.168-1.361-.801-2.5-1.86-3.301-3.306-.775-1.393-1.225-2.888-1.299-4.482-.02-.385.094-.522.477-.592a4.7 4.7 0 0 1 1.53-.038c2.131.311 3.946 1.264 5.467 2.774.868.86 1.525 1.887 2.202 2.89.72 1.066 1.494 2.082 2.48 2.915.348.291.626.513.892.677-.802.09-2.14.109-3.055-.615zm1.001-6.44a.306.306 0 0 1 .415-.287.3.3 0 0 1 .113.074.3.3 0 0 1 .086.214c0 .17-.136.307-.308.307a.303.303 0 0 1-.306-.307m3.11 1.596c-.2.081-.4.151-.591.16a1.25 1.25 0 0 1-.798-.254c-.274-.23-.47-.358-.551-.758a1.7 1.7 0 0 1 .015-.588c.07-.327-.007-.537-.238-.727-.188-.156-.426-.199-.689-.199a.6.6 0 0 1-.254-.078.253.253 0 0 1-.114-.358 1 1 0 0 1 .192-.21c.356-.202.767-.136 1.146.016.352.144.618.408 1.001.782.392.451.462.576.685.915.176.264.336.536.446.848.066.194-.02.353-.25.45';
// The owner's own mark for the OpenAI family (traced from the owner's image, 2026-11-16; fill-rule evenodd), viewBox 0 0 24 24
export const OPENAI_PATH =
  'M10.48 2.00L9.36 2.24L8.08 2.88L6.88 4.08L6.64 4.64L6.48 4.72L6.48 4.96L6.24 5.44L5.52 5.60L5.36 5.76L5.12 5.76L4.64 6.00L4.56 6.16L4.24 6.24L4.00 6.56L3.84 6.56L3.28 7.12L3.28 7.28L2.88 7.68L2.48 8.48L2.40 8.96L2.32 8.96L2.24 9.84L2.16 9.84L2.16 10.96L2.24 10.96L2.32 11.76L2.48 11.92L2.56 12.40L2.80 12.88L2.96 12.96L3.04 13.28L3.44 13.68L3.36 14.08L3.28 14.08L3.28 14.48L3.20 14.48L3.20 16.24L3.28 16.24L3.44 17.12L3.92 18.08L4.08 18.16L4.08 18.32L4.96 19.28L5.12 19.28L5.52 19.68L6.16 20.00L6.64 20.08L6.80 20.24L8.00 20.32L8.00 20.40L9.20 20.24L9.84 20.96L10.00 20.96L10.56 21.44L11.12 21.60L11.28 21.76L11.52 21.76L11.52 21.84L12.32 21.92L12.32 22.00L13.52 22.00L14.64 21.76L15.92 21.12L17.12 19.92L17.36 19.36L17.52 19.28L17.52 19.04L17.76 18.56L18.48 18.40L18.64 18.24L18.88 18.24L19.36 18.00L19.44 17.84L19.76 17.76L20.00 17.44L20.16 17.44L20.72 16.88L20.72 16.72L21.12 16.32L21.52 15.52L21.60 15.04L21.68 15.04L21.76 14.16L21.84 14.16L21.84 13.04L21.76 13.04L21.68 12.24L21.52 12.08L21.44 11.60L21.20 11.12L21.04 11.04L20.96 10.72L20.56 10.32L20.64 9.92L20.72 9.92L20.72 9.52L20.80 9.52L20.80 7.76L20.72 7.76L20.56 6.88L20.08 5.92L19.92 5.84L19.92 5.68L19.04 4.72L18.88 4.72L18.48 4.32L17.84 4.00L17.36 3.92L17.20 3.76L16.00 3.68L16.00 3.60L14.80 3.76L14.16 3.04L14.00 3.04L13.44 2.56L12.88 2.40L12.72 2.24L12.48 2.24L12.48 2.16L11.68 2.08L11.68 2.00ZM10.64 4.00L11.76 4.08L11.76 4.16L12.40 4.32L12.48 4.48L12.64 4.48L12.64 4.64L11.76 5.04L11.68 5.20L10.72 5.68L10.64 5.84L10.08 6.08L9.76 6.40L9.04 6.72L9.04 6.96L9.28 7.20L9.92 8.48L10.32 8.40L10.40 8.24L11.12 7.92L11.20 7.76L11.76 7.52L11.84 7.36L12.64 6.96L12.72 6.80L14.64 5.84L14.88 5.84L15.12 5.68L16.40 5.68L16.40 5.76L16.96 5.84L17.44 6.08L17.52 6.24L17.68 6.24L18.40 7.04L18.64 7.52L18.72 8.00L18.80 8.00L18.80 8.88L18.16 8.56L18.08 8.40L17.60 8.24L17.52 8.08L15.36 7.04L15.12 6.80L14.88 6.88L14.80 7.20L14.64 7.28L14.00 8.56L15.44 9.28L15.52 9.44L16.64 9.92L16.72 10.08L17.20 10.24L17.28 10.40L17.60 10.48L17.68 10.64L18.32 10.96L19.20 11.76L19.76 12.88L19.84 13.84L19.76 13.84L19.68 14.64L19.36 15.28L19.20 15.36L19.20 15.52L18.40 16.24L18.08 16.32L18.00 12.00L16.00 12.00L16.08 16.72L16.00 16.72L15.92 17.76L15.84 17.76L15.60 18.56L14.80 19.44L14.64 19.44L14.56 19.60L14.08 19.84L13.36 19.92L13.36 20.00L12.24 19.92L12.24 19.84L11.60 19.68L11.52 19.52L11.36 19.52L11.36 19.36L12.24 18.96L12.32 18.80L13.28 18.32L13.36 18.16L13.92 17.92L14.24 17.60L14.96 17.28L14.96 17.04L14.72 16.80L14.08 15.52L13.68 15.60L13.60 15.76L12.88 16.08L12.80 16.24L12.24 16.48L11.92 16.80L11.60 16.88L11.28 17.20L9.36 18.16L9.12 18.16L8.88 18.32L7.60 18.32L7.60 18.24L7.04 18.16L6.56 17.92L6.48 17.76L6.32 17.76L5.60 16.96L5.36 16.48L5.28 16.00L5.20 16.00L5.20 15.12L5.84 15.44L5.92 15.60L6.40 15.76L6.48 15.92L8.64 16.96L8.88 17.20L9.12 17.12L9.20 16.80L9.36 16.72L10.00 15.44L8.56 14.72L8.48 14.56L7.36 14.08L7.28 13.92L6.80 13.76L6.72 13.60L6.40 13.52L6.32 13.36L5.68 13.04L4.80 12.24L4.24 11.12L4.16 10.16L4.24 10.16L4.32 9.36L4.64 8.72L4.80 8.64L4.80 8.48L5.60 7.76L5.92 7.68L6.00 12.00L8.00 12.00L7.92 7.28L8.00 7.28L8.08 6.24L8.16 6.24L8.40 5.44L9.20 4.56L9.36 4.56L9.44 4.40L9.92 4.16L10.64 4.08Z';

// Gemma's four-point star, redrawn (owner's choice, 2026-11-16): concave sides made of four quarter-circle arcs
export const GEMMA_PATH = 'M12 1.5 A10.5 10.5 0 0 0 22.5 12 A10.5 10.5 0 0 0 12 22.5 A10.5 10.5 0 0 0 1.5 12 A10.5 10.5 0 0 0 12 1.5 Z';

const PATH: Record<EffortFamily, string> = { anthropic: ANTHROPIC_PATH, openai: OPENAI_PATH, google: GOOGLE_PATH, responses: DEEPSEEK_PATH, gemma: GEMMA_PATH };

/** A 16×16 monochrome mark (fill = currentColor), aria-hidden */
export function providerIcon(f: EffortFamily, cls = 'ais-pi'): SVGSVGElement {
  const s = document.createElementNS(SVGNS, 'svg');
  s.setAttribute('viewBox', '0 0 24 24');
  s.setAttribute('width', '16');
  s.setAttribute('height', '16');
  s.setAttribute('fill', 'currentColor');
  s.setAttribute('aria-hidden', 'true');
  s.setAttribute('focusable', 'false');
  s.setAttribute('class', cls);
  const p = document.createElementNS(SVGNS, 'path');
  p.setAttribute('d', PATH[f]);
  if (f === 'openai') p.setAttribute('fill-rule', 'evenodd');
  s.appendChild(p);
  return s;
}
