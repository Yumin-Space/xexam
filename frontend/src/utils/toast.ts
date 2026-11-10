let timer: number | undefined;

/** Toast bar at the bottom of the screen. ms defaults to 3.4 seconds. */
export function toast(msg: string, ms = 3400): void {
  const t = document.getElementById('toast');
  if (!t) return;
  t.textContent = msg;
  t.classList.add('show');
  window.clearTimeout(timer);
  timer = window.setTimeout(() => t.classList.remove('show'), ms);
}
