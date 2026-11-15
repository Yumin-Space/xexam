// Saving a downloaded file (v1.6 bank download).

/** Saves a downloaded blob under `name`: an object URL + a temporary <a download> clicked programmatically (desktop
 *  browsers, iPhone / iPad Safari 13+, Android Chrome). Without `download` support it falls back to location.assign(url).
 *  The URL is revoked after 60 s (iOS needs it alive while the download sheet opens). */
export function saveFile(blob: Blob, name: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  if ('download' in a) {
    a.href = url;
    a.download = name;
    a.rel = 'noopener';
    a.style.display = 'none';
    document.body.appendChild(a);
    a.click();
    a.remove();
  } else {
    location.assign(url);
  }
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}
