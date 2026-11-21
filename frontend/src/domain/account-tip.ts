// v2.2: the text of the phone account tip: who is signed in (name and role, already worded), then the AI+ state line when
// the account has a badge. Two lines; the tip keeps the line break (styles/shell.css `.acct-tipwrap > .tip`).
export function joinAccountTip(who: string, aiTip: string | null): string {
  return aiTip ? `${who}\n${aiTip}` : who;
}
