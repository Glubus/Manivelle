// Counts frames, shows a "get ready" banner for half a second, and answers the `status`
// request. The `frame` payload is typed by the declarations generated from Rust's `Frame`.
import { ctx, setTimeout } from "manivelle:env";

let frameCount = 0;
let lastTick = -1;
let banner = "get ready";
setTimeout(() => {
    banner = "playing";
}, 500);

ctx.on("frame", (frame) => {
    frameCount += 1;
    lastTick = frame.tick;
});
ctx.on(
    "status",
    () => `hud: ${(globalThis as any).gameTitle} ${banner} frames=${frameCount} tick=${lastTick}`,
);

export function frames(): number {
    return frameCount;
}
export function bannerText(): string {
    return banner;
}
