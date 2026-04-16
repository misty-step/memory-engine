export function assertNever(value: never): never {
  throw new Error(`Unreachable: ${JSON.stringify(value)}`);
}
