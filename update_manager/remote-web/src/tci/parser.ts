export type TciCommand = {
  name: string;
  args: string[];
  raw: string;
};

export function parseTciText(text: string): TciCommand[] {
  return text
    .split(';')
    .map((part) => part.trim())
    .filter((part) => part.length > 0)
    .map(parseTciCommand);
}

export function parseTciCommand(raw: string): TciCommand {
  const colon = raw.indexOf(':');
  if (colon < 0) {
    return {
      name: raw.trim().toLowerCase(),
      args: [],
      raw,
    };
  }

  const name = raw.slice(0, colon).trim().toLowerCase();
  const argText = raw.slice(colon + 1);
  const args = argText.split(',').map((arg) => arg.trim());

  return { name, args, raw };
}

export function booleanArg(value: string | undefined): boolean | null {
  if (value == null) return null;
  const normalized = value.trim().toLowerCase();
  if (['1', 'true', 'on', 'yes'].includes(normalized)) return true;
  if (['0', 'false', 'off', 'no'].includes(normalized)) return false;
  return null;
}

export function numericArg(value: string | undefined): number | null {
  if (value == null) return null;
  const parsed = Number(value.trim());
  return Number.isFinite(parsed) ? parsed : null;
}

export function trailingArg(args: readonly string[]): string | undefined {
  return args.length > 0 ? args[args.length - 1] : undefined;
}
