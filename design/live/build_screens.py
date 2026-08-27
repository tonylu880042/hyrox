# -*- coding: utf-8 -*-
"""Generates apps/hub-server/static/training.html.

The page is a shell: CSS (including the station glyph masks sliced from icons/*.png)
plus a renderer that draws whatever the hub pushes over /ws. No business logic lives
here -- every value on screen arrives already derived (CLAUDE.md 6, 29)."""
import io, os, base64
from PIL import Image
import numpy as np

OUT = "../../apps/hub-server/static/training.html"
MASK_SIDE, MASK_MARGIN = 256, 0.06

def _mask(path):
    """Glyph is saturated yellow on a flat dark plate; max(R,G) separates them
    with the antialiased edges intact. Returns base64 PNG, white + alpha."""
    a = np.asarray(Image.open(path).convert("RGB")).astype(float)
    alpha = np.clip((np.maximum(a[:, :, 0], a[:, :, 1]) - 30.0) / 205.0, 0, 1)
    h, w = alpha.shape
    inner = int(MASK_SIDE * (1 - 2 * MASK_MARGIN))
    sc = min(inner / w, inner / h)
    nw, nh = max(1, int(round(w * sc))), max(1, int(round(h * sc)))
    lay = Image.fromarray((alpha * 255).astype("uint8"), "L").resize((nw, nh), Image.LANCZOS)
    canvas = Image.new("L", (MASK_SIDE, MASK_SIDE), 0)
    canvas.paste(lay, ((MASK_SIDE - nw) // 2, (MASK_SIDE - nh) // 2))
    out = Image.merge("RGBA", (Image.new("L", (MASK_SIDE, MASK_SIDE), 255),) * 3 + (canvas,))
    buf = io.BytesIO(); out.save(buf, "PNG", optimize=True)
    return base64.b64encode(buf.getvalue()).decode()

MASKS = {os.path.splitext(f)[0]: _mask("icons/" + f)
         for f in sorted(os.listdir("icons")) if f.endswith(".png")}

mask_css = "\n".join(
    '        .pg-%s{-webkit-mask-image:url(data:image/png;base64,%s);'
    'mask-image:url(data:image/png;base64,%s);}' % (k, v, v) for k, v in MASKS.items())

head = io.open("leaderboard.html", encoding="utf-8").read().split("</head>")[0]
head = head.replace("<title>HYROX Live Leaderboard</title>", "<title>HYROX Training Class</title>")
head = head.replace("""        /* Rows share the leftover height evenly so nothing dead-spaces above the footer */
        .leaderboard-row {
            flex: 1 1 0;
            min-height: 0;
        }""",
"""        .class-grid { display: grid; grid-template-columns: repeat(4, 1fr);
            grid-template-rows: repeat(3, 1fr); gap: 12px; min-height: 0; }
        .athlete-card {
            position: relative; overflow: hidden;
            background-color: #131314; border: 1px solid #343535; border-left: 6px solid #FEE400;
            display: flex; flex-direction: column; justify-content: space-between;
            padding: 20px 24px; min-height: 0;
        }
        .athlete-card.transition { border-color: #7DF4FF; border-left-color: #7DF4FF; }
        .athlete-card.done { border-left-color: #FEE400; opacity: .5; }
        .athlete-card.ready { border-left-color: #6B6B6B; opacity: .7; }
        .card-watermark { position: absolute; right: -14px; bottom: -22px; font-size: 176px;
            line-height: 1; color: #FEE400; opacity: .08; pointer-events: none; }
        .athlete-card.transition .card-watermark { color: #7DF4FF; opacity: .13; }
        .station-icon { font-size: 54px; line-height: 1; flex-shrink: 0; }
        /* Sliced from the supplied pictogram sheet, applied as a mask so currentColor
           still drives the fill at every size. */
        .pg { width: 1em; height: 1em; display: inline-block; background-color: currentColor;
            -webkit-mask-repeat: no-repeat; mask-repeat: no-repeat;
            -webkit-mask-size: contain; mask-size: contain;
            -webkit-mask-position: center; mask-position: center; }
""" + mask_css + """
        .station-name { font-size: 46px; line-height: .92; }
        .seg { flex: 1; height: 22px; background-color: #2a2a2a; }
        .seg.done { background-color: #FEE400; }
        .seg.now  { background-color: #FEE400; outline: 2px solid #FFF; }
        .seg.moving { background-color: #FFF; }
        .foot-stat { display: inline-flex; align-items: center; gap: 8px; }
        .foot-stat .material-symbols-outlined { font-size: 22px; }
        /* Freshness colours (ADR 0001 D5): the screen must distinguish "nobody is running"
           from "the hub stopped hearing readers". */
        .fresh-ok   { color: #22DD66; }
        .fresh-warn { color: #FFB300; }
        .fresh-bad  { color: #FF4A4A; }""")
head = head.replace("font-size: 18px;\n            letter-spacing: 0.12em;",
                    "font-size: 22px;\n            letter-spacing: 0.12em;")

body = r"""
<body class="flex flex-col font-label-sm text-on-surface">
<header class="relative w-full shrink-0 flex justify-between items-center px-margin-edge bg-surface border-b border-outline-variant h-[100px]">
<div class="flex items-center gap-6">
<h1 class="font-headline-md text-headline-md text-primary-fixed uppercase tracking-tighter">CENTRAL HUB</h1>
<div class="h-6 w-gutter bg-outline-variant"></div>
<div class="flex items-center gap-4">
<span id="session-name" class="font-label-sm text-label-sm text-primary uppercase font-bold">&nbsp;</span>
<span id="mode-badge" class="px-3 py-1 bg-primary-fixed text-on-primary text-xs uppercase tracking-widest font-bold">&nbsp;</span>
</div>
</div>
<div class="flex items-center gap-8">
<div class="flex items-center gap-2">
<span id="live-dot" class="w-3 h-3 rounded-full bg-[#6B6B6B]"></span>
<span id="live-label" class="font-label-sm text-label-sm text-primary uppercase font-bold">CONNECTING</span>
</div>
<div class="h-6 w-gutter bg-outline-variant"></div>
<span class="font-label-sm text-label-sm text-on-surface-variant uppercase foot-stat"><span class="material-symbols-outlined" style="font-size:22px;">sensors</span> <span id="readers">--</span> READERS ONLINE</span>
<span id="freshness" class="font-label-sm text-label-sm uppercase foot-stat fresh-bad"><span class="material-symbols-outlined" style="font-size:22px;">bolt</span> <span id="fresh-text">NO DATA</span></span>
<div class="h-6 w-gutter bg-outline-variant"></div>
<div class="flex items-center gap-3 text-on-surface-variant">
<span class="material-symbols-outlined" style="font-size:26px;">timer</span>
<span class="station-label" style="margin:0;">CLASS ELAPSED</span>
<span id="class-elapsed" class="font-telemetry-data text-[32px] font-bold text-primary tabular-nums tracking-tighter">--:--</span>
</div>
</div>
</header>
<main id="grid" class="w-full flex-grow min-h-0 overflow-hidden px-margin-edge py-[16px] class-grid bg-background"></main>
<footer class="relative w-full shrink-0 flex items-center gap-10 px-margin-edge bg-surface-container-lowest border-t border-outline-variant h-[56px]">
<span class="font-label-sm text-label-sm uppercase tracking-widest text-on-surface foot-stat"><span class="material-symbols-outlined" style="font-size:22px;">groups</span> IN CLASS <span id="f-in">--</span></span>
<span class="font-label-sm text-label-sm uppercase tracking-widest text-on-surface foot-stat"><span class="material-symbols-outlined" style="font-size:22px;">flag</span> FINISHED <span id="f-done">--</span></span>
<span class="font-label-sm text-label-sm uppercase tracking-widest text-on-surface foot-stat"><span class="material-symbols-outlined" style="font-size:22px;">route</span> COURSE <span id="f-course">--</span> STATIONS</span>
<span class="font-label-sm text-label-sm uppercase tracking-widest text-on-surface foot-stat"><span class="material-symbols-outlined" style="font-size:22px;">report</span> EXCEPTIONS <span id="f-exc">--</span></span>
</footer>
<script>
const $ = (id) => document.getElementById(id);

function clock(ms, tenths) {
  if (ms === null || ms === undefined) return "--:--";
  if (ms < 0) ms = 0;
  const t = Math.floor(ms / 100) % 10;
  const s = Math.floor(ms / 1000) % 60;
  const m = Math.floor(ms / 60000) % 60;
  const h = Math.floor(ms / 3600000);
  const p = (n) => String(n).padStart(2, "0");
  const base = h > 0 ? `${h}:${p(m)}:${p(s)}` : `${p(m)}:${p(s)}`;
  return tenths ? `${base}.${t}` : base;
}

function segments(a, courseLen) {
  let html = '<div style="display:flex;gap:3px;">';
  for (let k = 0; k < courseLen; k++) {
    let cls = "seg";
    if (k < a.completed) cls += " done";
    else if (k === a.completed && a.status !== "FINISHED") {
      cls += a.station_state === "INSIDE" ? " now" : " moving";
    }
    html += `<div class="${cls}"></div>`;
  }
  return html + "</div>";
}

function card(a, course) {
  const inside = a.station_state === "INSIDE";
  const finished = a.status === "FINISHED";
  const ready = a.status === "READY";

  let cls = "athlete-card";
  let glyph = "", label = "", colour = "#FFFFFF", sub = "";

  if (finished) {
    cls += " done"; colour = "#FEE400";
    glyph = '<span class="material-symbols-outlined station-icon" style="color:#FEE400;">flag</span>';
    label = "FINISHED";
    sub = `<div class="station-label" style="margin-top:12px;"><span class="material-symbols-outlined" style="font-size:22px;vertical-align:-4px;">check_circle</span> ALL ${course.length} STATIONS COMPLETE</div>`;
  } else if (ready) {
    cls += " ready"; colour = "#6B6B6B";
    glyph = '<span class="material-symbols-outlined station-icon" style="color:#6B6B6B;">hourglass_empty</span>';
    label = "READY";
    sub = '<div class="station-label" style="margin-top:12px;">WAITING FOR FIRST SCAN</div>';
  } else if (!inside) {
    cls += " transition"; colour = "#7DF4FF";
    glyph = '<span class="material-symbols-outlined station-icon" style="color:#7DF4FF;">directions_walk</span>';
    label = "TRANSITION";
    const next = a.next_station ? `MOVING TO ${a.next_station}` : "MOVING";
    sub = `<div style="display:flex;justify-content:space-between;align-items:baseline;margin-top:10px;">
      <span class="font-telemetry-data text-[46px] font-bold tabular-nums" style="color:#7DF4FF;">${clock(a.leg_ms, true)}</span>
      <span class="station-label" style="margin:0;"><span class="material-symbols-outlined" style="font-size:22px;vertical-align:-4px;">east</span> ${next}</span></div>`;
  } else {
    glyph = `<span class="station-icon pg pg-${a.station_key}" style="color:#FFFFFF;"></span>`;
    label = a.station;
    // Station split is what the hub can actually derive from entry/exit reads.
    // Work done inside the station is equipment telemetry the hub never sees.
    sub = `<div style="display:flex;justify-content:space-between;align-items:baseline;margin-top:10px;">
      <span class="font-telemetry-data text-[46px] font-bold tabular-nums text-primary">${clock(a.leg_ms, true)}</span>
      <span class="station-label" style="margin:0;">${a.plan || ""}</span></div>`;
  }

  const watermark = inside && a.station_key
    ? `<span class="card-watermark pg pg-${a.station_key}"></span>`
    : (finished
        ? '<span class="material-symbols-outlined card-watermark">flag</span>'
        : (ready ? "" : '<span class="material-symbols-outlined card-watermark">directions_walk</span>'));

  // The leg time is already the headline number above; repeating it here just adds noise.
  const legLabel = inside ? "IN STATION" : (finished ? "COMPLETE" : ready ? "NOT STARTED" : "IN TRANSITION");

  return `<div class="${cls}">
${watermark}
<div>
<div style="display:flex;justify-content:space-between;align-items:flex-start;">
<div class="font-athlete-name text-[40px] uppercase leading-none truncate">${a.name}</div>
<span class="font-telemetry-data text-[22px] text-on-surface-variant">${String(a.bib).padStart(2, "0")}</span>
</div>
<div style="display:flex;align-items:center;gap:14px;margin-top:8px;">
${glyph}<div class="font-headline-md station-name" style="color:${colour};">${label}</div>
</div>
${sub}
</div>
<div>
${segments(a, course.length)}
<div style="display:flex;justify-content:space-between;align-items:baseline;margin-top:12px;">
<span class="station-label" style="margin:0;"><span class="material-symbols-outlined" style="font-size:22px;vertical-align:-4px;">timer</span> ${legLabel}</span>
<span class="font-telemetry-data text-[50px] font-bold tabular-nums text-primary">${clock(a.elapsed_ms, false)}</span>
</div>
</div>
</div>`;
}

function render(s) {
  $("session-name").textContent = s.session_name;
  $("mode-badge").textContent = s.mode;
  $("readers").textContent = s.readers_online;
  $("class-elapsed").textContent = clock(s.class_elapsed_ms, false);
  $("f-in").textContent = s.in_class;
  $("f-done").textContent = s.finished;
  $("f-course").textContent = s.course.length;
  $("f-exc").textContent = s.exceptions;

  const age = s.last_event_age_ms;
  const box = $("freshness");
  box.classList.remove("fresh-ok", "fresh-warn", "fresh-bad");
  if (age === null || age === undefined) {
    box.classList.add("fresh-bad");
    $("fresh-text").textContent = "NO EVENTS YET";
  } else {
    box.classList.add(age < 10000 ? "fresh-ok" : age < 30000 ? "fresh-warn" : "fresh-bad");
    $("fresh-text").textContent = `LAST EVENT ${Math.floor(age / 1000)}s AGO`;
  }

  $("grid").innerHTML = s.course.length
    ? s.athletes.map((a) => card(a, s.course)).join("")
    : "";
}

function setLink(up) {
  $("live-dot").style.backgroundColor = up ? "#22DD66" : "#FF4A4A";
  $("live-dot").classList.toggle("animate-pulse", up);
  $("live-label").textContent = up ? "LIVE" : "DISCONNECTED";
  if (!up) {
    // A frozen screen must never be mistaken for a quiet gym (ADR 0001 D5).
    const box = $("freshness");
    box.classList.remove("fresh-ok", "fresh-warn");
    box.classList.add("fresh-bad");
    $("fresh-text").textContent = "LINK DOWN";
  }
}

function connect() {
  const ws = new WebSocket(`ws://${location.host}/ws`);
  ws.onopen = () => setLink(true);
  ws.onmessage = (e) => render(JSON.parse(e.data));
  ws.onclose = () => { setLink(false); setTimeout(connect, 1000); };
  ws.onerror = () => ws.close();
}
connect();
</script>
</body></html>"""

io.open(OUT, "w", encoding="utf-8").write(head + "</head>" + body)

# Review sheet for eyeballing the glyphs at size. Generated, not committed.
cells = "".join(
    '<div class="cell"><span class="ic pg pg-%s"></span><div class="lbl">%d. %s</div></div>'
    % (k, i + 1, k.replace("_", " ").upper()) for i, k in enumerate(MASKS))
io.open("icon-sheet.html", "w", encoding="utf-8").write(
    """<!DOCTYPE html><html><head><meta charset="utf-8"><title>HYROX station pictograms</title>
<style>body{margin:0;background:#0A0A0A;color:#fff;font-family:system-ui,sans-serif;width:800px;}
.grid{display:grid;grid-template-columns:repeat(4,1fr);gap:5px;padding:8px;}
.cell{background:#131314;border:1px solid #343535;padding:8px;text-align:center;}
.ic{display:block;font-size:170px;color:#FEE400;}
.pg{width:1em;height:1em;display:inline-block;background-color:currentColor;
    -webkit-mask-repeat:no-repeat;mask-repeat:no-repeat;-webkit-mask-size:contain;mask-size:contain;
    -webkit-mask-position:center;mask-position:center;}
.lbl{margin-top:5px;font-size:13px;letter-spacing:.1em;color:#9c9c9c;}
""" + mask_css + """</style></head><body><div class="grid">%s</div></body></html>""" % cells)

assert len(MASKS) == 8, MASKS.keys()
print("wrote %s (%d KB), masks: %d" % (OUT, len(head + body) // 1024, len(MASKS)))
