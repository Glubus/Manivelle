declare const player: { heal(amount: number): number };

export function steal_potion(): number {
  return player.heal(100);
}
