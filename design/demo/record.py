#!/usr/bin/env python3
"""把展示劇本錄成影片：真的跑一次系統，逐格截真的畫面，壓上字幕。

    python3 design/demo/record.py group-class

產出（在 design/demo/out/）：
    hyrox-<片名>-1080p.mp4         燒錄字幕
    hyrox-<片名>-1080p-nosub.mp4   無字幕版，配 captions-<片名>.srt

每一格的長度直接取自 captions.py，所以影片和 .srt 永遠對得上——字幕改秒數，
影片跟著改，不需要對時間碼。

這是**粗剪**：一句字幕一張真實截圖，沒有運鏡、沒有轉場、沒有配樂。
它證明畫面上的東西是真的，不取代發包單（docs/video-brief.md）裡的後製。

需要：headless Chrome、ffmpeg、Pillow、mosquitto、hub-server。
"""

import os
import subprocess
import time
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

from captions import SCRIPTS

HERE = Path(__file__).parent
OUT = HERE / "out"
FRAMES = OUT / "frames"
CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
HUB = "http://127.0.0.1:8730"
W, H = 1920, 1080

CJK = "/System/Library/Fonts/Hiragino Sans GB.ttc"
MONO = "/System/Library/Fonts/Menlo.ttc"

# 版面（1920x1080）。畫面內縮浮在黑底上，字卡壓在左下角 —— 對齊 FitRace 的做法，
# 字大、對比高、投影機上讀得到，不靠播放器的字幕功能。
ACCENT = (216, 245, 33)        # 螢幕本身的黃綠色
INSET = 46                     # 截圖四周留的黑邊
CARD_X, CARD_BOTTOM = 64, 56
CARD_PAD = (34, 26)
CARD_TEXT = 46
CHAPTER_SIZE = 116
FPS = 30
KEN_BURNS = 0.05          # 整段推近的幅度；再多就會讓人暈
DRIFT = 40                 # 推近時橫向漂移的像素


def font(size, mono=False):
    return ImageFont.truetype(MONO if mono else CJK, size)


def glow(im, box, colour, spread=14):
    """字卡外圍的光暈。畫幾層遞減透明度的框，比做模糊便宜也夠看。"""
    d = ImageDraw.Draw(im, "RGBA")
    x0, y0, x1, y1 = box
    for i in range(spread, 0, -1):
        a = int(46 * (1 - i / spread) ** 2)
        d.rounded_rectangle([x0 - i, y0 - i, x1 + i, y1 + i], radius=10 + i,
                            outline=colour + (a,), width=2)


def trim_bottom(im, pad=30):
    """只裁掉下方的空白。

    這些頁面是流動版面：只有四位選手時，下半部整片是空的，縮到 1080p 之後字就
    小到讀不出來。裁掉底部空白、保留整個寬度 —— 只裁一個方向，才不會把欄位切掉
    （試過裁欄位：墨水最多的那一欄是進度條，名字和成績全被切掉）。
    """
    a = np.asarray(im.convert("RGB")).astype(int)
    bg = a[2, 2]
    ink = (np.abs(a - bg).max(axis=2) > 18).sum(axis=1)
    rows = np.where(ink > max(24, im.width * 0.012))[0]
    if not len(rows):
        return im
    bottom = min(im.height, rows[-1] + pad)
    if bottom > im.height * 0.92:            # 本來就填滿，不動它
        return im
    return im.crop((0, 0, im.width, bottom))


def stage(shot_img):
    """把截圖擺到 1920x1080 的黑底上，四周留邊，加一道細框。"""
    frame = Image.new("RGB", (W, H), (8, 8, 8))
    inner_w, inner_h = W - INSET * 2, H - INSET * 2
    fitted = shot_img.copy()
    fitted.thumbnail((inner_w, inner_h), Image.LANCZOS)
    x, y = (W - fitted.width) // 2, (H - fitted.height) // 2
    if fitted.height < H * 0.72:      # 窄長的畫面往上擺，把下方留給字卡
        y = int(H * 0.30 - fitted.height / 2)
    frame.paste(fitted, (x, y))
    d = ImageDraw.Draw(frame, "RGBA")
    d.rectangle([x - 1, y - 1, x + fitted.width, y + fitted.height], outline=(64, 64, 64, 255))
    return frame


def caption_layer(text):
    """左下角字卡，畫在透明圖層上。

    分開畫是因為畫面會緩慢推近（Ken Burns），字卡不能跟著一起縮放 —— 會糊，
    而且字在動的畫面上更難讀。推近只作用在截圖，字卡最後才疊上去。
    """
    im = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(im, "RGBA")
    lines = text.split("\n")
    f = font(CARD_TEXT)
    tw = max(d.textlength(l, font=f) for l in lines)
    th = len(lines) * (CARD_TEXT + 16) - 16
    x0, y1 = CARD_X, H - CARD_BOTTOM
    x1, y0 = x0 + tw + CARD_PAD[0] * 2, y1 - th - CARD_PAD[1] * 2
    glow(im, (x0, y0, x1, y1), ACCENT)
    d.rounded_rectangle([x0, y0, x1, y1], radius=10, fill=(0, 0, 0, 235),
                        outline=ACCENT + (255,), width=2)
    y = y0 + CARD_PAD[1]
    for line in lines:
        d.text((x0 + CARD_PAD[0], y), line, font=f, fill=(255, 255, 255))
        y += CARD_TEXT + 16
    return im


def caption(im, text):
    """靜態合成版（給截圖檢查用）。影片走 caption_layer + overlay。"""
    out = im.convert("RGBA")
    out.alpha_composite(caption_layer(text))
    return out.convert("RGB")


def chapter(number, title):
    """段落卡：整頁黑底，編號在上，標題在下。"""
    im = Image.new("RGB", (W, H), (8, 8, 8))
    d = ImageDraw.Draw(im)
    d.text((CARD_X + 8, H // 2 - 150), number, font=font(46, mono=True), fill=ACCENT)
    d.text((CARD_X, H // 2 - 80), title, font=font(CHAPTER_SIZE), fill=(255, 255, 255))
    y = H // 2 + 80
    d.line([(CARD_X + 4, y), (CARD_X + 4 + 220, y)], fill=ACCENT, width=6)
    return im


def title_card(text):
    im = Image.new("RGB", (W, H), (8, 8, 8))
    d = ImageDraw.Draw(im)
    # 產品名稱可能很長，字級縮到放得下為止，不要換行也不要溢出。
    size = 96
    f = font(size)
    while d.textlength(text, font=f) > W - 240 and size > 48:
        size -= 4
        f = font(size)
    d.text(((W - d.textlength(text, font=f)) / 2, H / 2 - size), text, font=f, fill=(255, 255, 255))
    sub = "場館端主機 · 賽事與課程"
    fs = font(40)
    d.text(((W - d.textlength(sub, font=fs)) / 2, H / 2 + 40), sub, font=fs, fill=(150, 150, 150))
    return im


def sh(cmd):
    """跑一段 shell，回傳 stdout。展示的按鈕都在 group-class.sh 裡，這裡只負責按。"""
    return subprocess.run(
        ["bash", "-c",
         f"cd {HERE.parent.parent} && source {HERE}/group-class.sh >/dev/null 2>&1; "
         f"source {HERE}/reliability.sh >/dev/null 2>&1; "
         f"source {HERE}/competition.sh >/dev/null 2>&1; {cmd}"],
        capture_output=True, text=True,
    ).stdout.strip()


def shoot(url, path, size=(1400, 800)):
    """用**小的**瀏覽器視窗、2 倍像素密度擷取。

    這些頁面的列高是固定像素，所以視窗開得越大，字在畫面裡就越小 —— 用 2560 寬去截
    再縮成 1080p，字會小到讀不出來。改成小視窗（字佔比大）＋ 2 倍密度（縮放後仍銳利）。
    """
    subprocess.run(
        [CHROME, "--headless=new", "--disable-gpu", "--hide-scrollbars",
         "--force-device-scale-factor=2", "--virtual-time-budget=4000",
         f"--window-size={size[0]},{size[1]}", f"--screenshot={path}", url],
        capture_output=True,
    )
    return Image.open(path).convert("RGB")


def wrapped(text):
    """終端機輸出常常比畫面寬。截掉會讓觀眾看不到重點，所以折行。"""
    out = []
    for line in text.split("\n"):
        # 中文字寬約是等寬字的兩倍，用權重估寬度，超過就折。
        limit, width, buf = 96, 0, ""
        for ch in line:
            w = 2 if ord(ch) > 0x2E80 else 1
            if width + w > limit:
                out.append(buf)
                buf, width = "", 0
            buf += ch
            width += w
        out.append(buf)
    return out


def card(text, title=""):
    """終端機輸出的字卡。內容是真的跑出來的 stdout，只是排版過。"""
    im = Image.new("RGB", (W, H), (8, 8, 8))
    d = ImageDraw.Draw(im, "RGBA")
    d.rounded_rectangle([INSET, INSET, W - INSET, H - INSET], radius=12,
                        fill=(14, 14, 14, 255), outline=(64, 64, 64, 255))
    d.text((INSET + 54, INSET + 44), title, font=font(46), fill=ACCENT)
    d.line([(INSET + 54, INSET + 122), (W - INSET - 54, INSET + 122)], fill=(60, 60, 60), width=2)
    mono, han = font(32, mono=True), font(32)
    y = INSET + 172
    for line in wrapped(text)[:18]:
        # Menlo 沒有中文字，含中文的那一行改用中文字型（等寬只好放棄）。
        f = han if any(ord(c) > 0x2E80 for c in line) else mono
        d.text((INSET + 54, y), line, font=f, fill=(228, 228, 228))
        y += 46
    return im


def clip(plain_png, cap_png, seconds, index, out):
    """把一張靜圖變成會動的一段。

    緩慢推近（Ken Burns，5% 以內、方向交替），字卡最後才疊上去所以不跟著縮放，
    開頭 0.3 秒淡入 —— 一句一句硬切會像投影片，這是最便宜的「活起來」。
    """
    frames = max(2, int(seconds * FPS))
    zoom_in = index % 2 == 0
    z = (f"1+{KEN_BURNS}*on/({frames}-1)" if zoom_in
         else f"1+{KEN_BURNS}-{KEN_BURNS}*on/({frames}-1)")
    pan = "iw/2-(iw/zoom/2)"
    drift = f"{pan}+{'-' if zoom_in else ''}{DRIFT}*on/({frames}-1)"
    chain = (
        f"[0:v]scale={W * 2}:{H * 2},"
        f"zoompan=z='{z}':x='{drift}':y='ih/2-(ih/zoom/2)':d={frames}:s={W}x{H}:fps={FPS},"
        f"fade=t=in:d=0.3[bg]"
    )
    if cap_png:
        chain += (f";[1:v]format=rgba,fade=t=in:st=0.25:d=0.45:alpha=1[cap];"
                  f"[bg][cap]overlay=0:0[v]")
    else:
        chain += ";[bg]null[v]"
    cmd = ["ffmpeg", "-y", "-loop", "1", "-i", str(plain_png)]
    if cap_png:
        cmd += ["-loop", "1", "-i", str(cap_png)]
    cmd += ["-filter_complex", chain, "-map", "[v]", "-t", f"{seconds}",
            "-c:v", "libx264", "-preset", "medium", "-crf", "20",
            "-pix_fmt", "yuv420p", str(out)]
    subprocess.run(cmd, capture_output=True)


def build(name, shots):
    cues = [c for _act, cs in SCRIPTS[name] for c in cs]
    if len(cues) != len(shots):
        sys.exit(f"{name}: {len(cues)} 則字幕但 {len(shots)} 格畫面，對不起來")

    FRAMES.mkdir(parents=True, exist_ok=True)
    plain, subbed = [], []
    for i, ((dur, text), shot) in enumerate(zip(cues, shots)):
        if shot.get("hub"):
            hub(shot["hub"])
        if shot.get("do"):
            shot.setdefault("out", sh(shot["do"]))

        kind = shot["shot"]
        if kind == "chapter":
            im, cap = chapter(shot["number"], text), None
        elif kind == "title":
            im, cap = title_card(text), None
        elif kind == "card":
            im, cap = card(shot.get("out", ""), shot.get("title", "")), caption_layer(text)
        else:
            raw = shoot(f"{HUB}{kind}", str(FRAMES / f"raw{i:03}.png"),
                        shot.get("size", (1400, 800)))
            im, cap = stage(trim_bottom(raw)), caption_layer(text)

        p = FRAMES / f"p{i:03}.png"
        im.save(p)
        cap_png = None
        if cap is not None:
            cap_png = FRAMES / f"c{i:03}.png"
            cap.save(cap_png)

        sub_clip, plain_clip = FRAMES / f"s{i:03}.mp4", FRAMES / f"n{i:03}.mp4"
        clip(p, cap_png, dur, i, sub_clip)
        clip(p, None, dur, i, plain_clip)
        subbed.append(sub_clip)
        plain.append(plain_clip)
        print(f"  {i + 1:2}/{len(cues)}  {dur}s  {text.splitlines()[0]}")

    total = sum(d for d, _t in cues)
    for clips, suffix in ((subbed, ""), (plain, "-nosub")):
        listing = OUT / f"{name}{suffix}.txt"
        listing.write_text("".join(f"file '{c}'\n" for c in clips))
        mp4 = OUT / f"hyrox-{name}-1080p{suffix}.mp4"
        subprocess.run(
            ["ffmpeg", "-y", "-f", "concat", "-safe", "0", "-i", str(listing),
             "-c", "copy", str(mp4)],
            capture_output=True,
        )
        if suffix == "":
            music(mp4, total)
        print(mp4)


def music(mp4, seconds):
    """配上背景音樂。找不到音檔就算了 —— 影片本身不靠它成立。

    NCS 的曲子是免費授權但**要求標示出處**，發佈時請把曲名與作者寫在說明欄
    （docs/video-brief.md §7）。
    """
    tracks = sorted((HERE.parent.parent / "assets").glob("*.mp3"))
    if not tracks:
        return
    scored = mp4.with_name(mp4.stem + "-tmp.mp4")
    fade = 4
    subprocess.run(
        ["ffmpeg", "-y", "-i", str(mp4), "-i", str(tracks[0]),
         "-filter_complex",
         f"[1:a]atrim=0:{seconds},afade=t=in:st=0:d=2,"
         f"afade=t=out:st={seconds - fade}:d={fade},volume=0.14[a]",
         "-map", "0:v", "-map", "[a]", "-c:v", "copy", "-c:a", "aac", "-b:a", "160k",
         "-shortest", str(scored)],
        capture_output=True,
    )
    if scored.exists() and scored.stat().st_size > 0:
        scored.replace(mp4)


# --- 團課版：24 格，對應 captions.py 的 group-class -------------------------
# 每一格 = 先按一顆按鈕（可省略），再截一張圖。按鈕全部來自 group-class.sh。
GROUP_CLASS = [
    {"shot": "/workout"},
    {"do": "act1_plan", "shot": "/workout"},
    {"shot": "/workout"},
    {"do": "act1b_readonly", "shot": "card", "title": "改系統課表 → 被擋下"},
    {"do": "act2_open", "shot": "card", "title": "開課：課表編譯成 10 個關卡"},
    {"shot": "/live", "size": (1500, 840)},
    {"shot": "/live", "size": (1500, 840)},
    {"do": "act3_checkin", "shot": "card", "title": "報到：四個人、四隻手環"},
    {"shot": "/checkin"},
    {"shot": "/checkin"},
    {"do": "act4_start", "shot": "/live", "size": (1500, 840)},
    {"do": "act5_floor", "shot": "card", "title": "上課：刷卡進站、出站"},
    {"shot": "/live", "size": (1500, 840)},
    {"shot": "/live", "size": (1500, 840)},
    {"do": "act6_pause", "shot": "card", "title": "暫停：課堂時鐘"},
    {"do": "elapsed", "shot": "card", "title": "三秒後：數字不動"},
    {"do": "act6_resume", "shot": "/live", "size": (1500, 840)},
    {"do": "act7_misread", "shot": "card", "title": "誤刷：刷到還沒走到的站"},
    {"do": "act7_stages", "shot": "card", "title": "系統的判斷：順序不對"},
    {"shot": "/live", "size": (1500, 840)},
    {"do": "act7b_void", "shot": "card", "title": "作廢：沒有理由不給過"},
    {"do": "act7c_trail", "shot": "card", "title": "原始刷卡與稽核紀錄"},
    {"do": "act8_end; act9_results", "shot": "card", "title": "收工與成績"},
    {"shot": "/leaderboard", "size": (1150, 640)},
]

# --- 可靠性版：24 格 -------------------------------------------------------
# 這一支會**把主機關掉再開起來**，所以格與格之間有真的停機。按鈕在 reliability.sh。
RELIABILITY = [
    {"shot": "/live", "size": (1500, 840)},
    {"shot": "/live", "size": (1500, 840)},
    {"shot": "/live", "size": (1500, 840)},
    {"do": "act_stray_tag", "shot": "card", "title": "沒有登記的手環刷過 SkiErg"},
    {"shot": "/checkin"},
    {"do": "act_claim", "shot": "card", "title": "現場報名並綁定"},
    {"shot": "/live", "size": (1500, 840)},
    {"do": "mark_now; act_walkin_time", "shot": "card", "title": "計時從刷卡那一刻算起"},
    {"do": "act_kill", "hub": "stop", "shot": "card", "title": "把主機關掉"},
    {"do": "act_refused", "shot": "card", "title": "主機沒了"},
    {"do": "act_publish_while_down", "shot": "card", "title": "主機躺著，讀卡機還在刷"},
    {"do": "echo '這一筆已經在 broker 的佇列裡，等主機回來。'", "shot": "card",
     "title": "關機期間送出的那一筆"},
    {"hub": "start", "do": "act_resumed_log", "shot": "card", "title": "重新開機：接回原本那一堂課"},
    {"do": "act_arrived", "shot": "card", "title": "關機期間那一筆，進來了"},
    {"do": "act_two_clocks", "shot": "card", "title": "紀錄裡的兩個時間"},
    {"do": "act_two_clocks", "shot": "card", "title": "成績只用刷卡時間"},
    {"shot": "/live", "size": (1500, 840)},
    {"do": "act_resend", "shot": "card", "title": "讀卡機重送"},
    {"do": "act_still_one", "shot": "card", "title": "資料庫裡仍然只有一筆"},
    {"shot": "/leaderboard", "size": (1150, 640)},
    {"shot": "/leaderboard", "size": (1150, 640)},
    {"shot": "/leaderboard", "size": (1150, 640)},
    {"do": "act_offline_assets", "shot": "card", "title": "畫面的每一個檔案都由主機供應"},
    {"shot": "/live?lang=zh-Hans"},
]


# --- 合併版：賽制設定 → 比賽過程 → 課程規劃 → 課程執行 -----------------------
# 按鈕來自 competition.sh 與 group-class.sh。比賽先跑完並結案，才輪得到團課
# （同一時間只能有一堂課在跑）。
DIRECTOR = "-H 'x-operator-device: 賽事平板'"
OVERVIEW = [
    {"shot": "title"},

    {"shot": "chapter", "number": "01"},
    {"do": "curl -s -X POST %s -H 'content-type: application/json' -d '{\"reason\":\"開賽事\"}' "
           "$HUB/api/operator/session/cancel > /dev/null" % DIRECTOR,
     "shot": "/workout", "size": (1250, 760)},
    {"do": "race_track", "shot": "/workout?template=race-sprint-3", "size": (1250, 820)},
    {"do": "race_open", "shot": "/workout?view=class", "size": (1250, 820)},
    {"do": "race_checkin", "shot": "/checkin", "size": (1250, 700)},
    {"do": "race_start", "shot": "/live", "size": (2560, 1200)},

    {"shot": "chapter", "number": "02"},
    {"do": "race_wave1", "shot": "/live", "size": (2560, 1200)},
    {"do": "race_wave2", "shot": "/live", "size": (2560, 1200)},
    {"do": "race_wave3", "shot": "/live", "size": (2560, 1200)},
    {"shot": "/live", "size": (2560, 1200)},
    {"shot": "/leaderboard", "size": (1150, 640)},
    {"shot": "/leaderboard", "size": (1150, 640)},

    {"shot": "chapter", "number": "03"},
    {"do": "curl -s -X POST %s $HUB/api/operator/session/complete > /dev/null" % DIRECTOR,
     "shot": "/workout", "size": (1250, 760)},
    {"do": "act1_plan", "shot": "/workout?template=sys-power", "size": (1250, 820)},
    {"shot": "/workout?template=coach-power", "size": (1250, 820)},
    {"do": "act2_open", "shot": "/workout?view=class", "size": (1250, 820)},
    {"shot": "/workout?view=class", "size": (1250, 820)},

    {"shot": "chapter", "number": "04"},
    {"do": "act3_checkin", "shot": "/checkin", "size": (1250, 700)},
    {"do": "act4_start", "shot": "/live", "size": (2560, 1200)},
    {"do": "act5_floor", "shot": "/live", "size": (2560, 1200)},
    {"do": "act6_pause", "shot": "/workout?view=class", "size": (1250, 820)},
    {"do": "act6_resume; act8_end", "shot": "/workout?view=class", "size": (1250, 820)},
    {"shot": "/leaderboard", "size": (1150, 640)},

    {"shot": "title"},
]

SHOTS = {"group-class": GROUP_CLASS, "reliability": RELIABILITY, "overview": OVERVIEW}

if __name__ == "__main__":
    name = sys.argv[1] if len(sys.argv) > 1 else "group-class"
    if name not in SHOTS:
        sys.exit(f"還沒有這支片的分鏡：{name}（可用：{', '.join(SHOTS)}）")
    OUT.mkdir(exist_ok=True)
    os.environ.setdefault("HYROX_DB_FILE", "hyrox.db")
    build(name, SHOTS[name])
