declare const player: { heal(amount: number): number };

export function potion(): number {
  return player.heal(25);
}
