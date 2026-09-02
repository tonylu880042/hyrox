# -*- coding: utf-8 -*-
"""Generates apps/hub-server/static/training.html.

The page is a shell: CSS (including the station glyph masks sliced from icons/*.png)
plus a renderer that draws whatever the hub pushes over /ws. No business logic lives
here -- every value on screen arrives already derived (CLAUDE.md 6, 29)."""
import io
import re, os, base64
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
# The shared interface dictionary (roadmap M7), served locally by the hub. Both screens read
# their labels from the same file, so a label cannot say one thing on the projector and
# another on the coach's tablet.
# Stylesheet and fonts from the hub, not a CDN (CLAUDE.md 31). `app.css` is the CDN's own
# output captured once; see design/README.md.
head = re.sub(r'<script src="https://cdn\.tailwindcss\.com[^>]*></script>', '', head)
head = re.sub(r'<link[^>]+fonts\.google[^>]+>', '', head)
head = re.sub(r'<link[^>]+fonts\.gstatic[^>]+>', '', head)
head = re.sub(r'<script id="tailwind-config">.*?</script>', '', head, flags=re.S)
head += '<link rel="stylesheet" href="/fonts.css"><link rel="stylesheet" href="/app.css">'
# The tab icon, so a wall of open screens is identifiable at a glance.
head += '<link rel="icon" type="image/svg+xml" href="/favicon.svg">'
head += '<script src="/i18n.js"></script>'

head = head.replace("<title>HYROX Live Leaderboard</title>", "<title>HYROX Training Class</title>")
head = head.replace("""        /* Rows share the leftover height evenly so nothing dead-spaces above the footer */
        .leaderboard-row {
            flex: 1 1 0;
            min-height: 0;
        }""",
"""        /* The projector's own size. Every type size here was chosen for a 30-foot
           room, so on a smaller screen the picture is scaled rather than reflowed: a laptop
           shows the same screen, smaller, instead of a cropped one. At 2560x1440 the scale
           is exactly 1, so nothing about the venue's projector changes. */
        html { background: #0A0A0A; overflow: hidden; }
        body { width: 2560px; height: 1440px; transform-origin: top left; }
        /* minmax(0, 1fr), not 1fr: a grid track's min-width defaults to auto, so one long
           name pushes the whole row wider than the screen instead of being clipped by its
           own card. That was the overflow. */
        .class-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr));
            grid-template-rows: repeat(3, minmax(0, 1fr)); gap: 12px; min-height: 0; }
        /* A belt for the braces: if more cards than a page ever reach this grid, the extra
           rows take a real height instead of squeezing the explicit rows to zero and
           stacking every card on top of the others -- which is what a 30-person class used
           to do. Pagination is the fix; this is what stops it being invisible if it breaks. */
        .class-grid { grid-auto-rows: minmax(120px, 1fr); }
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
<!-- The venue's own mark leads: this wall belongs to the gym, and the system that drives
     it is the supplier's credit in the footer. Hidden until one is uploaded, so a venue
     without a logo gets no empty box. -->
<!-- Inline style, not utility classes: app.css is a captured Tailwind build, so an
     arbitrary class like h-[48px] simply does not exist in it and silently does nothing.
     Learned by watching a 600px logo eat the header. -->
<img id="venue-logo" class="hidden" alt=""
     style="height:52px;max-width:260px;object-fit:contain;flex-shrink:0;margin-right:20px"/>
<h1 class="font-headline-md text-headline-md text-primary-fixed uppercase tracking-tight whitespace-nowrap" data-i18n="live.hub">HYROX TRAINING &amp; COMPETITION SYSTEM</h1>
<div class="h-6 w-gutter bg-outline-variant"></div>
<div class="flex items-center gap-4">
<span id="session-name" class="font-label-sm text-label-sm text-primary uppercase font-bold whitespace-nowrap">&nbsp;</span>
<span id="mode-badge" class="px-3 py-1 bg-primary-fixed text-on-primary text-xs uppercase tracking-widest font-bold">&nbsp;</span>
</div>
</div>
<div class="flex items-center gap-8 whitespace-nowrap">
<div class="flex items-center gap-2">
<span id="live-dot" class="w-3 h-3 rounded-full" style="background-color:#6B6B6B"></span>
<span id="live-label" class="font-label-sm text-label-sm text-primary uppercase font-bold" data-i18n="live.connecting">CONNECTING</span>
</div>
<div class="h-6 w-gutter bg-outline-variant"></div>
<span class="font-label-sm text-label-sm text-on-surface-variant uppercase foot-stat"><span class="material-symbols-outlined" style="font-size:22px;">sensors</span> <span id="readers">--</span> <span data-i18n="live.readersOnline">READERS ONLINE</span></span>
<span id="freshness" class="font-label-sm text-label-sm uppercase foot-stat fresh-bad"><span class="material-symbols-outlined" style="font-size:22px;">bolt</span> <span id="fresh-text" data-i18n="live.noData">NO DATA</span></span>
<div class="h-6 w-gutter bg-outline-variant"></div>
<div class="flex items-center gap-3 text-on-surface-variant">
<span class="material-symbols-outlined" style="font-size:26px;">timer</span>
<span class="station-label" style="margin:0;" data-i18n="live.classElapsed">CLASS ELAPSED</span>
<span id="class-elapsed" class="font-telemetry-data text-[32px] font-bold text-primary tabular-nums tracking-tighter">--:--</span>
</div>
<div class="flex items-center gap-3 text-on-surface-variant">
<span id="page-indicator" class="hidden font-telemetry-data text-[32px] font-bold tabular-nums tracking-tighter px-3 py-1 border border-outline-variant">1 / 1</span>
</div>
</div>
</header>
<main id="grid" class="w-full flex-grow min-h-0 overflow-hidden px-margin-edge py-[16px] class-grid bg-background"></main>
<footer class="relative w-full shrink-0 flex items-center gap-10 px-margin-edge bg-surface-container-lowest border-t border-outline-variant h-[56px]">
<span class="font-label-sm text-label-sm uppercase tracking-widest text-on-surface foot-stat"><span class="material-symbols-outlined" style="font-size:22px;">groups</span> <span data-i18n="live.inClass">IN CLASS</span> <span id="f-in">--</span></span>
<span class="font-label-sm text-label-sm uppercase tracking-widest text-on-surface foot-stat"><span class="material-symbols-outlined" style="font-size:22px;">flag</span> <span data-i18n="live.finished">FINISHED</span> <span id="f-done">--</span></span>
<span class="font-label-sm text-label-sm uppercase tracking-widest text-on-surface foot-stat"><span class="material-symbols-outlined" style="font-size:22px;">route</span> <span data-i18n="live.course">COURSE</span> <span id="f-course">--</span> <span data-i18n="live.stations">STATIONS</span></span>
<span class="font-label-sm text-label-sm uppercase tracking-widest text-on-surface foot-stat"><span class="material-symbols-outlined" style="font-size:22px;">report</span> <span data-i18n="live.exceptions">EXCEPTIONS</span> <span id="f-exc">--</span></span>
<!-- Whose system this is. Right-hand end of the footer, in the muted colour: a projector
     screen belongs to the class on it, not to us, so the mark is present and quiet. -->
<span class="ml-auto font-label-sm text-label-sm tracking-widest text-on-surface-variant">uCareMedi.com</span>
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

// ── Translating what the snapshot calls things ───────────────────────────────────────────
//
// The snapshot carries `station` as the venue's station key -- "WALL BALLS" -- because that
// string is simultaneously a course step, a reader's registration and this page's pictogram
// slug (ADR 0008). It is an identifier, so it is translated here at the point of display and
// nowhere else. The map comes from /api/exercises, which pairs each key with an
// `Exercise.code`; an unmapped key falls back to itself.
let STATION_NAME = {};

async function loadStationNames() {
  try {
    const res = await fetch("/api/exercises");
    for (const e of (await res.json()).exercises) {
      const key = "ex." + e.code;
      const text = I18N.t(key);
      STATION_NAME[e.station_key] = text === key ? e.display_name : text;
    }
  } catch (err) {
    // No names is survivable; a blank screen is not. Station keys are readable English.
    console.warn("exercise names unavailable", err);
  }
}

function stationName(key) {
  return key ? STATION_NAME[key] || key : "";
}

// `plan` arrives pre-formatted by the hub -- "800 M", "50 REPS", "3:00 MIN" (see
// `target_label` in crates/application/src/live.rs). Only the trailing unit token needs a
// language; the number is the number. A token this map does not know is left alone.
const PLAN_UNIT = { M: "METER", KM: "KILOMETER", REPS: "REPS", CAL: "CALORIE", MIN: "MINUTE", S: "SECOND" };

function planLabel(plan) {
  if (!plan) return "";
  const at = plan.lastIndexOf(" ");
  if (at < 0) return plan;
  const unit = PLAN_UNIT[plan.slice(at + 1)];
  return unit ? plan.slice(0, at) + " " + I18N.t("unit." + unit) : plan;
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
    label = I18N.t("live.finished");
    sub = `<div class="station-label" style="margin-top:12px;"><span class="material-symbols-outlined" style="font-size:22px;vertical-align:-4px;">check_circle</span> ${I18N.t("live.allComplete", course.length)}</div>`;
  } else if (ready) {
    cls += " ready"; colour = "#6B6B6B";
    glyph = '<span class="material-symbols-outlined station-icon" style="color:#6B6B6B;">hourglass_empty</span>';
    label = I18N.t("live.ready");
    sub = '<div class="station-label" style="margin-top:12px;">' + I18N.t("live.waitingFirstScan") + "</div>";
  } else if (!inside) {
    cls += " transition"; colour = "#7DF4FF";
    glyph = '<span class="material-symbols-outlined station-icon" style="color:#7DF4FF;">directions_walk</span>';
    label = I18N.t("live.transition");
    const next = a.next_station
      ? I18N.t("live.movingTo", stationName(a.next_station))
      : I18N.t("live.moving");
    sub = `<div style="display:flex;justify-content:space-between;align-items:baseline;margin-top:10px;">
      <span class="font-telemetry-data font-bold tabular-nums" style="font-size:46px;color:#7DF4FF;">${clock(a.leg_ms, true)}</span>
      <span class="station-label" style="margin:0;"><span class="material-symbols-outlined" style="font-size:22px;vertical-align:-4px;">east</span> ${next}</span></div>`;
  } else {
    glyph = `<span class="station-icon pg pg-${a.station_key}" style="color:#FFFFFF;"></span>`;
    label = stationName(a.station);
    // Station split is what the hub can actually derive from entry/exit reads.
    // Work done inside the station is equipment telemetry the hub never sees.
    sub = `<div style="display:flex;justify-content:space-between;align-items:baseline;margin-top:10px;">
      <span class="font-telemetry-data font-bold tabular-nums text-primary" style="font-size:46px;">${clock(a.leg_ms, true)}</span>
      <span class="station-label" style="margin:0;">${planLabel(a.plan)}</span></div>`;
  }

  const watermark = inside && a.station_key
    ? `<span class="card-watermark pg pg-${a.station_key}"></span>`
    : (finished
        ? '<span class="material-symbols-outlined card-watermark">flag</span>'
        : (ready ? "" : '<span class="material-symbols-outlined card-watermark">directions_walk</span>'));

  // The leg time is already the headline number above; repeating it here just adds noise.
  const legLabel = I18N.t(
    inside ? "live.inStation" : finished ? "live.complete" : ready ? "live.notStarted" : "live.inTransition"
  );

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
    $("fresh-text").textContent = I18N.t("live.noEvents");
  } else {
    box.classList.add(age < 10000 ? "fresh-ok" : age < 30000 ? "fresh-warn" : "fresh-bad");
    $("fresh-text").textContent = I18N.t("freshness.ago", `${Math.floor(age / 1000)}s`);
  }

  SNAPSHOT = s;
  renderPage();
}

/// The grid holds one page. More athletes than that and the screen rotates rather than
/// shrinking the cards: this is read from across a gym, and a card small enough to fit
/// thirty people is a card nobody can read (M6 follow-up, decided with the user).
///
/// Order is the roster's own, never "most interesting first": a projector that reorders
/// itself makes an athlete's card move while their coach is looking at it.
let PAGE_SIZE = 12;
let LAYOUT = { columns: 4, rows: 3 };
/// How long a page is held. The venue sets it on /settings -- a long thin gym reads slower
/// than a studio -- and this is only the value used until that answer arrives.
let PAGE_MS = 10000;
let SNAPSHOT = null;
let PAGE = 0;

function pages() {
  if (!SNAPSHOT || !SNAPSHOT.athletes.length) return 1;
  return Math.max(1, Math.ceil(SNAPSHOT.athletes.length / PAGE_SIZE));
}

function renderPage() {
  const s = SNAPSHOT;
  if (!s) return;
  // Clamped rather than wrapped: people leaving the roster must not silently skip a page.
  if (PAGE >= pages()) PAGE = 0;
  const from = PAGE * PAGE_SIZE;
  const shown = s.athletes.slice(from, from + PAGE_SIZE);
  $("grid").innerHTML = s.course.length ? shown.map((a) => card(a, s.course)).join("") : "";

  // The indicator is only drawn when there is something to page through. "1 / 1" on a
  // twelve-person class is noise that makes an operator wonder what they are missing.
  const label = $("page-indicator");
  if (pages() > 1) {
    label.textContent = `${PAGE + 1} / ${pages()}`;
    label.classList.remove("hidden");
  } else {
    label.classList.add("hidden");
  }
}

/// The rotation runs on its own timer rather than inside the snapshot loop: snapshots
/// arrive four times a second, and a page that turned on data arriving would flicker.
let pageTimer = setInterval(turnPage, PAGE_MS);

function turnPage() {
  if (pages() > 1) {
    PAGE = (PAGE + 1) % pages();
    renderPage();
  }
}

/// Re-read the venue's setting periodically, so changing it on a tablet reaches the
/// projector without anybody walking over to reload the screen.
async function loadSettings() {
  try {
    const settings = await (await fetch("/api/settings")).json();
    if (settings.live_page_ms && settings.live_page_ms !== PAGE_MS) {
      PAGE_MS = settings.live_page_ms;
      clearInterval(pageTimer);
      pageTimer = setInterval(turnPage, PAGE_MS);
    }
    // The grid's shape comes from the same list the picker offers, so the projector can
    // never be asked for a layout nobody chose the card proportions for.
    const chosen = (settings.page_layouts || []).find((l) => l.size === settings.live_page_size);
    if (chosen && chosen.size !== PAGE_SIZE) {
      PAGE_SIZE = chosen.size;
      LAYOUT = chosen;
      const grid = $("grid");
      grid.style.gridTemplateColumns = `repeat(${chosen.columns}, minmax(0, 1fr))`;
      grid.style.gridTemplateRows = `repeat(${chosen.rows}, minmax(0, 1fr))`;
      PAGE = 0;
      renderPage();
    }
  } catch (e) {
    // The screen keeps rotating on the value it already has. A settings read that fails is
    // not a reason to stop showing the class.
  }
}
loadSettings();
setInterval(loadSettings, 30000);

/// The venue's logo, if there is one. A 404 is an answer, not a failure: the header simply
/// leads with the system name instead. Re-checked with the settings, so uploading one on a
/// tablet reaches the projector without anybody reloading it.
function loadLogo() {
  const img = $("venue-logo");
  const probe = new Image();
  probe.onload = () => { img.src = probe.src; img.classList.remove("hidden"); };
  probe.onerror = () => img.classList.add("hidden");
  probe.src = "/api/logo?t=" + Date.now();
}
loadLogo();
setInterval(loadLogo, 30000);

function setLink(up) {
  $("live-dot").style.backgroundColor = up ? "#22DD66" : "#FF4A4A";
  $("live-dot").classList.toggle("animate-pulse", up);
  $("live-label").textContent = I18N.t(up ? "live.live" : "live.disconnected");
  if (!up) {
    // A frozen screen must never be mistaken for a quiet gym (ADR 0001 D5).
    const box = $("freshness");
    box.classList.remove("fresh-ok", "fresh-warn");
    box.classList.add("fresh-bad");
    $("fresh-text").textContent = I18N.t("live.linkDown");
  }
}

function connect() {
  const ws = new WebSocket(`ws://${location.host}/ws`);
  ws.onopen = () => setLink(true);
  ws.onmessage = (e) => render(JSON.parse(e.data));
  ws.onclose = () => { setLink(false); setTimeout(connect, 1000); };
  ws.onerror = () => ws.close();
}
// Labels first, then the socket: the screen must never flash English before it settles.
// There is no language switcher here on purpose -- a projector is not an interactive screen.
// Pin it with /live?lang=zh-Hans, which is remembered on that machine afterwards.
// Fit the projector's fixed canvas into whatever is actually showing it. Five lines rather
// than a responsive rewrite: this screen is one composition sized for a room, and scaling
// keeps every proportion it depends on -- including the pictogram masks, which are sliced
// at a fixed size and would misregister if the type reflowed.
function fitScreen() {
  const scale = Math.min(innerWidth / 2560, innerHeight / 1440);
  // Centred both ways: a window that is not 16:9 gets even bars rather than one fat one
  // at the bottom, which reads as a broken layout rather than a deliberate fit.
  const left = Math.max(0, (innerWidth - 2560 * scale) / 2);
  const top = Math.max(0, (innerHeight - 1440 * scale) / 2);
  document.body.style.transform = `translate(${left}px, ${top}px) scale(${scale})`;
}
addEventListener("resize", fitScreen);
fitScreen();

I18N.apply();
loadStationNames();
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
