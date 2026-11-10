// Spawns a wave every second of game time, keeps its wave count across reloads, and
// adds the stored high score to it. `game` and `manivelle:env` are typed by the
// declarations generated from the Rust contracts (`manivelle.d.ts`).
import { ctx, setTimeout } from "manivelle:env";
import { game } from "game";

const VERSION = 1;

let waves = (ctx.hot.data as { waves: number } | undefined)?.waves ?? 0;
ctx.hot.save(() => ({ waves }));

function scheduleWave(): void {
    setTimeout(() => {
        waves += 1;
        scheduleWave();
    }, 1000);
}
scheduleWave();

// A group shares one global object: the HUD reads this.
(globalThis as any).gameTitle = "Asteroids";

ctx.on("status", () => `spawner v${VERSION}: waves=${waves}`);

export function version(): number {
    return VERSION;
}
export function wavesSpawned(): number {
    return waves;
}
export async function bestScore(): Promise<number> {
    return (await game.highScore()) + waves * 100;
}
