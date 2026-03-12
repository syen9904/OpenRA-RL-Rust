import init, { ReplayViewer } from './pkg/openra_wasm.js';

const canvas = document.getElementById('canvas');
const ctx = canvas.getContext('2d');
const status = document.getElementById('status');
const btnLoad = document.getElementById('btn-load');
const btnPlay = document.getElementById('btn-play');
const btnStep = document.getElementById('btn-step');
const speedSlider = document.getElementById('speed');
const speedVal = document.getElementById('speed-val');
const replayInput = document.getElementById('replay-file');
const mapInput = document.getElementById('map-file');

let viewer = null;
let playing = false;
let animFrameId = null;
let replayBytes = null;
let mapBytes = null;
let lastSnapshot = null;

// Player colors (index by player actor ID mod colors.length)
const PLAYER_COLORS = [
    '#888888', // 0: neutral/world (grey)
    '#888888', // 1: Neutral (grey)
    '#888888', // 2: Creeps (grey)
    '#ffcc00', // 3: Player 1 (yellow)
    '#e94560', // 4: Player 2 (red)
    '#4488ff', // 5: Everyone (blue)
];

function getPlayerColor(playerIndex) {
    return PLAYER_COLORS[playerIndex] || '#ffffff';
}

// Enable load button when both files are selected
function checkFiles() {
    btnLoad.disabled = !(replayBytes && mapBytes);
}

replayInput.addEventListener('change', async (e) => {
    const file = e.target.files[0];
    if (file) {
        replayBytes = new Uint8Array(await file.arrayBuffer());
        status.textContent = `Replay loaded: ${file.name} (${replayBytes.length} bytes)`;
    }
    checkFiles();
});

mapInput.addEventListener('change', async (e) => {
    const file = e.target.files[0];
    if (file) {
        mapBytes = new Uint8Array(await file.arrayBuffer());
        status.textContent = `Map loaded: ${file.name} (${mapBytes.length} bytes)`;
    }
    checkFiles();
});

btnLoad.addEventListener('click', () => {
    try {
        viewer = new ReplayViewer(replayBytes, mapBytes);
        status.textContent = `Loaded! ${viewer.total_frames()} frames. Use Play or Step.`;
        btnPlay.disabled = false;
        btnStep.disabled = false;
        lastSnapshot = JSON.parse(viewer.snapshot_json());
        render(lastSnapshot);
    } catch (e) {
        status.textContent = `Error: ${e}`;
    }
});

btnPlay.addEventListener('click', () => {
    if (playing) {
        playing = false;
        btnPlay.textContent = 'Play';
        if (animFrameId) cancelAnimationFrame(animFrameId);
    } else {
        playing = true;
        btnPlay.textContent = 'Pause';
        runLoop();
    }
});

btnStep.addEventListener('click', () => {
    if (!viewer) return;
    stepOnce();
});

speedSlider.addEventListener('input', () => {
    speedVal.textContent = speedSlider.value;
});

function stepOnce() {
    const ok = viewer.tick();
    if (!ok) {
        playing = false;
        btnPlay.textContent = 'Play';
        btnPlay.disabled = true;
        btnStep.disabled = true;
        status.textContent = 'Replay finished.';
        return false;
    }
    lastSnapshot = JSON.parse(viewer.snapshot_json());
    render(lastSnapshot);
    status.textContent = `Frame ${viewer.current_frame()} / ${viewer.total_frames()} | Tick ${lastSnapshot.tick}`;
    return true;
}

function runLoop() {
    if (!playing || !viewer) return;
    const framesPerRaf = parseInt(speedSlider.value);
    for (let i = 0; i < framesPerRaf; i++) {
        if (!stepOnce()) return;
    }
    animFrameId = requestAnimationFrame(runLoop);
}

function render(snapshot) {
    const w = canvas.width;
    const h = canvas.height;
    ctx.fillStyle = '#0a3d0a';
    ctx.fillRect(0, 0, w, h);

    if (!snapshot || !snapshot.actors.length) return;

    // Map cell coords to canvas pixels
    const mapW = snapshot.map_width || 128;
    const mapH = snapshot.map_height || 128;
    const scaleX = w / mapW;
    const scaleY = h / mapH;
    const scale = Math.min(scaleX, scaleY);
    const offsetX = (w - mapW * scale) / 2;
    const offsetY = (h - mapH * scale) / 2;

    for (const actor of snapshot.actors) {
        const sx = offsetX + actor.x * scale;
        const sy = offsetY + actor.y * scale;
        const color = getPlayerColor(actor.owner);

        ctx.fillStyle = color;

        if (actor.kind === 'Building') {
            // Buildings: larger rectangle
            const size = Math.max(3, scale * 2);
            ctx.fillRect(sx - size / 2, sy - size / 2, size, size);
        } else if (actor.kind === 'Mcv') {
            // MCV: medium circle
            const r = Math.max(3, scale * 0.8);
            ctx.beginPath();
            ctx.arc(sx, sy, r, 0, Math.PI * 2);
            ctx.fill();
        } else if (actor.kind === 'Tree') {
            // Trees: small dark green squares
            ctx.fillStyle = '#2d5a2d';
            const size = Math.max(2, scale * 0.6);
            ctx.fillRect(sx - size / 2, sy - size / 2, size, size);
        } else if (actor.kind === 'Mine') {
            // Mines: small orange diamonds
            ctx.fillStyle = '#cc8833';
            const size = Math.max(2, scale * 0.5);
            ctx.save();
            ctx.translate(sx, sy);
            ctx.rotate(Math.PI / 4);
            ctx.fillRect(-size / 2, -size / 2, size, size);
            ctx.restore();
        }
    }

    // Draw player cash overlay
    ctx.fillStyle = '#ffffff';
    ctx.font = '12px monospace';
    let yOff = 16;
    for (const p of snapshot.players) {
        const color = getPlayerColor(p.index);
        ctx.fillStyle = color;
        ctx.fillText(`P${p.index}: $${p.cash}`, 8, yOff);
        yOff += 16;
    }
}

// Initialize WASM
await init();
status.textContent = 'WASM loaded. Select replay and map files.';
