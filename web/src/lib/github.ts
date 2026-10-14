const REFUSED = "GitHub refused the connection. Start again from Settings.";

const REASONS = new Map([
  ["no-code", "GitHub did not send a code."],
  ["expired", "That connection attempt expired. Start again from Settings."],
  ["refused", REFUSED],
]);

export function failureSentence(reason: string | null): string {
  return REASONS.get(reason ?? "") ?? REFUSED;
}
