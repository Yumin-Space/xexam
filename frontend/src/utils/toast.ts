let timer: number | undefined;

/** 屏幕底部的提示条。ms 默认 3.4 秒。 */
export function toast(msg: string, ms = 3400): void {
  const t = document.getElementById('toast');
  if (!t) return;
  t.textContent = msg;
  t.classList.add('show');
  window.clearTimeout(timer);
  timer = window.setTimeout(() => t.classList.remove('show'), ms);
}
