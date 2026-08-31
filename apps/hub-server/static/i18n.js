// Interface-layer translations for /live and /workout (roadmap M7).
//
// Served locally at /i18n.js, not from a CDN: the hub must be useful with no internet
// (CLAUDE.md 31), and a screen whose labels failed to load is worse than an English one.
//
// ── What is NOT here, on purpose ─────────────────────────────────────────────────────────
//
// * Anything a person typed. Class names, athlete names, template names, correction
//   reasons and device names are content, not interface, and travel as they were entered.
// * Identifiers. `station_key` ("WALL BALLS") is simultaneously a course step, a reader's
//   registration and the slug the live screen picks a pictogram from -- translating it
//   would unmap every reader and blank every icon (ADR 0008). Same for `Exercise.code`,
//   every enum's wire value, and `ErrorBody.error`. Those are contract; this file is
//   presentation.
// * The API's `message` field. `docs/api.md` §6 says branch on the `error` code and treat
//   `message` as something for whoever reads a log. So the codes are translated here and
//   the server's English messages are never shown to a coach.
//
// Traditional and Simplified are maintained separately rather than converted. Gym
// vocabulary genuinely differs, and a converter would also mangle the user's own content.

(function (global) {
  "use strict";

  const DICT = {
    "zh-Hant": {
      // --- shared chrome
      "nav.workouts": "課表", "nav.builder": "編輯器", "nav.class": "課程",
      "nav.live": "大螢幕", "app.title": "HYROX 課表編排",
      "freshness.none": "尚無事件", "freshness.ago": "最後事件 {0} 前",
      "action.cancel": "取消", "action.ok": "確定",

      // --- workout list
      "list.search": "搜尋課表", "list.create": "＋ 建立課表", "list.empty": "尚無課表。",
      "list.system": "系統", "list.mine": "自建",
      "list.view": "檢視", "list.edit": "編輯", "list.duplicate": "複製",
      "list.use": "使用", "list.delete": "刪除",
      "list.meta": "{0} · {1} 個動作{2} · v{3}", "list.minutes": " · {0} 分鐘",

      // --- builder
      "builder.name": "課表名稱", "builder.category": "類型", "builder.minutes": "分鐘",
      "builder.readonly": "這是系統課表，無法修改。請按「複製」建立可編輯的版本。",
      "builder.addBlock": "＋ 新增區塊", "builder.addExercise": "＋ 新增動作",
      "builder.removeBlock": "移除區塊", "builder.rounds": "回合",
      "builder.blockName": "訓練區塊",
      "builder.save": "儲存", "builder.saveAs": "另存新檔…", "builder.use": "用這份開課",
      "builder.newName": "新課表",

      // --- create class
      "class.title": "建立課程", "class.workout": "課表", "class.name": "課程名稱",
      "class.coach": "教練", "class.ends": "結束方式", "class.minutes": "分鐘",
      "class.save": "儲存草稿",
      "class.today": "今天的課程",
      "class.todayNote": "這裡的調整<b>只影響今天</b>，不會動到你建立的課表。",
      "finish.COACH_DECIDES": "由教練決定", "finish.CLASS_DURATION": "固定時間後結束",
      "finish.COURSE_COMPLETE": "跑完課表即結束",
      "cmd.ready": "準備就緒", "cmd.start": "開始上課", "cmd.pause": "暫停",
      "cmd.resume": "繼續", "cmd.complete": "結束課程", "cmd.cancel": "取消課程",

      // --- questions
      "ask.device": "為這台裝置命名", "ask.deviceDefault": "櫃檯平板",
      "ask.copyName": "複本名稱", "ask.copySuffix": "{0}（複本）",
      "ask.saveAs": "另存為", "ask.deleteReason": "刪除這份課表的原因？",
      "ask.cancelReason": "取消這堂課的原因？",

      // --- results
      "toast.duplicated": "已複製，這份是你的，可以編輯。",
      "toast.deleted": "已刪除。", "toast.saved": "已儲存為第 {0} 版。",
      "toast.savedNew": "已另存為新課表。",
      "toast.classCreated": "課程已建立為草稿。可在下方調整今晚的目標，然後開始上課。",
      "toast.retargeted": "只影響今天，已儲存的課表未變動。",
      "toast.pickWorkout": "請先選一份課表。",
      "toast.deviceNeeded": "這台裝置要先命名才能做任何變更。",

      // --- error codes (docs/api.md §6)
      "err.OPERATOR_REQUIRED": "這台裝置還沒命名，無法寫入。",
      "err.INVALID_BODY": "送出的內容格式不正確。",
      "err.UNKNOWN_SESSION": "找不到這堂課程。",
      "err.UNKNOWN_ATHLETE": "這位學員不在本堂名單內。",
      "err.UNKNOWN_EVENT": "找不到這筆事件。",
      "err.UNKNOWN_TEMPLATE": "找不到這份課表。",
      "err.ILLEGAL_TRANSITION": "目前狀態不能做這個動作。",
      "err.HAS_INTERPRETED_EVENTS": "這堂課已經有成績紀錄，不能退回草稿。",
      "err.SESSION_NOT_EDITABLE": "課程開始後就不能改課表內容了。",
      "err.NO_FINISH_RULE": "這堂課沒有設定結束規則，無法手動結束。",
      "err.TAG_ALREADY_BOUND": "這個腳環已經綁在別人身上。",
      "err.ATHLETE_ALREADY_BOUND": "這位學員已經有腳環了，請用重新綁定。",
      "err.NOT_BOUND": "沒有可以變更的綁定。",
      "err.REASON_REQUIRED": "這個動作會變更紀錄，必須填寫原因。",
      "err.TEMPLATE_NOT_EDITABLE": "系統課表不能修改或刪除，請先複製一份。",
      "err.TEMPLATE_NOT_RUNNABLE": "這份課表無法開課：內容不完整或包含無法計時的區塊。",
      "err.CLASS_IN_PROGRESS": "已經有一堂課在進行中，請先結束或取消。",
      "err.STORAGE_FAILED": "資料庫寫入失敗。",
      "err.unknown": "發生未預期的錯誤。",

      // --- live screen
      "live.hub": "中央主機", "live.course": "課表", "live.stations": "站點",
      "live.classElapsed": "已進行", "live.readersOnline": "讀取器上線",
      "live.inClass": "上課人數", "live.finished": "已完成", "live.exceptions": "異常",
      "live.connecting": "連線中", "live.noData": "無資料", "live.live": "連線正常",
      "live.disconnected": "連線中斷", "live.linkDown": "訊號中斷",
      "live.noEvents": "尚無事件", "live.notStarted": "尚未開始",
      "live.inStation": "站內", "live.inTransition": "轉換中", "live.moving": "移動中",
      "live.complete": "完成", "live.ready": "準備", "live.transition": "轉換",
      "live.waitingFirstScan": "等待第一次刷卡", "live.allComplete": "全部 {0} 站完成",
      "live.movingTo": "前往 {0}",

      // --- exercise display names, keyed by Exercise.code (never by station_key)
      "ex.RUN": "跑步", "ex.SKIERG": "滑雪機", "ex.ROWERG": "划船機",
      "ex.SLED_PUSH": "推雪橇", "ex.SLED_PULL": "拉雪橇",
      "ex.BURPEE_BROAD_JUMP": "波比跳遠", "ex.FARMERS_CARRY": "農夫走路",
      "ex.SANDBAG_LUNGE": "沙袋弓箭步", "ex.WALL_BALL": "藥球上拋",

      // --- units
      "unit.METER": "公尺", "unit.KILOMETER": "公里", "unit.REPS": "次",
      "unit.SECOND": "秒", "unit.MINUTE": "分", "unit.CALORIE": "大卡",

      // --- categories
      "cat.FOUNDATIONAL": "基礎", "cat.ENGINE": "心肺", "cat.POWER": "力量",
      "cat.COMPLETE": "全項", "cat.RACE_SIMULATION": "模擬賽", "cat.CUSTOM": "自訂",

      // --- block types
      "blk.SEQUENTIAL": "循序", "blk.ROUNDS": "回合", "blk.AMRAP": "AMRAP",
      "blk.INTERVAL": "間歇", "blk.ZONE_ROTATION": "分區輪換",

      // --- stage status
      "stage.PENDING": "待進行", "stage.READY": "下一站", "stage.ACTIVE": "進行中",
      "stage.COMPLETED": "已完成", "stage.SKIPPED": "略過", "stage.DNF": "未完成",

      // --- session status
      "st.DRAFT": "草稿", "st.READY": "準備就緒", "st.RUNNING": "進行中",
      "st.PAUSED": "已暫停", "st.COMPLETED": "已結束", "st.CANCELLED": "已取消",
    },

    "zh-Hans": {
      "nav.workouts": "课表", "nav.builder": "编辑器", "nav.class": "课程",
      "nav.live": "大屏幕", "app.title": "HYROX 课表编排",
      "freshness.none": "尚无事件", "freshness.ago": "最后事件 {0} 前",
      "action.cancel": "取消", "action.ok": "确定",

      "list.search": "搜索课表", "list.create": "＋ 创建课表", "list.empty": "尚无课表。",
      "list.system": "系统", "list.mine": "自建",
      "list.view": "查看", "list.edit": "编辑", "list.duplicate": "复制",
      "list.use": "使用", "list.delete": "删除",
      "list.meta": "{0} · {1} 个动作{2} · v{3}", "list.minutes": " · {0} 分钟",

      "builder.name": "课表名称", "builder.category": "类型", "builder.minutes": "分钟",
      "builder.readonly": "这是系统课表，无法修改。请点「复制」创建可编辑的版本。",
      "builder.addBlock": "＋ 添加区块", "builder.addExercise": "＋ 添加动作",
      "builder.removeBlock": "移除区块", "builder.rounds": "回合",
      "builder.blockName": "训练区块",
      "builder.save": "保存", "builder.saveAs": "另存为…", "builder.use": "用这份开课",
      "builder.newName": "新课表",

      "class.title": "创建课程", "class.workout": "课表", "class.name": "课程名称",
      "class.coach": "教练", "class.ends": "结束方式", "class.minutes": "分钟",
      "class.save": "保存草稿",
      "class.today": "今天的课程",
      "class.todayNote": "这里的调整<b>只影响今天</b>，不会改动你创建的课表。",
      "finish.COACH_DECIDES": "由教练决定", "finish.CLASS_DURATION": "固定时间后结束",
      "finish.COURSE_COMPLETE": "跑完课表即结束",
      "cmd.ready": "准备就绪", "cmd.start": "开始上课", "cmd.pause": "暂停",
      "cmd.resume": "继续", "cmd.complete": "结束课程", "cmd.cancel": "取消课程",

      "ask.device": "为这台设备命名", "ask.deviceDefault": "前台平板",
      "ask.copyName": "副本名称", "ask.copySuffix": "{0}（副本）",
      "ask.saveAs": "另存为", "ask.deleteReason": "删除这份课表的原因？",
      "ask.cancelReason": "取消这堂课的原因？",

      "toast.duplicated": "已复制，这份是你的，可以编辑。",
      "toast.deleted": "已删除。", "toast.saved": "已保存为第 {0} 版。",
      "toast.savedNew": "已另存为新课表。",
      "toast.classCreated": "课程已创建为草稿。可在下方调整今晚的目标，然后开始上课。",
      "toast.retargeted": "只影响今天，已保存的课表未改动。",
      "toast.pickWorkout": "请先选一份课表。",
      "toast.deviceNeeded": "这台设备要先命名才能做任何更改。",

      "err.OPERATOR_REQUIRED": "这台设备还没命名，无法写入。",
      "err.INVALID_BODY": "提交的内容格式不正确。",
      "err.UNKNOWN_SESSION": "找不到这堂课程。",
      "err.UNKNOWN_ATHLETE": "这位学员不在本堂名单内。",
      "err.UNKNOWN_EVENT": "找不到这条事件。",
      "err.UNKNOWN_TEMPLATE": "找不到这份课表。",
      "err.ILLEGAL_TRANSITION": "当前状态不能做这个操作。",
      "err.HAS_INTERPRETED_EVENTS": "这堂课已经有成绩记录，不能退回草稿。",
      "err.SESSION_NOT_EDITABLE": "课程开始后就不能改课表内容了。",
      "err.NO_FINISH_RULE": "这堂课没有设置结束规则，无法手动结束。",
      "err.TAG_ALREADY_BOUND": "这个脚环已经绑在别人身上。",
      "err.ATHLETE_ALREADY_BOUND": "这位学员已经有脚环了，请用重新绑定。",
      "err.NOT_BOUND": "没有可以变更的绑定。",
      "err.REASON_REQUIRED": "这个操作会变更记录，必须填写原因。",
      "err.TEMPLATE_NOT_EDITABLE": "系统课表不能修改或删除，请先复制一份。",
      "err.TEMPLATE_NOT_RUNNABLE": "这份课表无法开课：内容不完整或包含无法计时的区块。",
      "err.CLASS_IN_PROGRESS": "已经有一堂课在进行中，请先结束或取消。",
      "err.STORAGE_FAILED": "数据库写入失败。",
      "err.unknown": "发生未预期的错误。",

      "live.hub": "中央主机", "live.course": "课表", "live.stations": "站点",
      "live.classElapsed": "已进行", "live.readersOnline": "读取器在线",
      "live.inClass": "上课人数", "live.finished": "已完成", "live.exceptions": "异常",
      "live.connecting": "连接中", "live.noData": "无数据", "live.live": "连接正常",
      "live.disconnected": "连接中断", "live.linkDown": "信号中断",
      "live.noEvents": "尚无事件", "live.notStarted": "尚未开始",
      "live.inStation": "站内", "live.inTransition": "转换中", "live.moving": "移动中",
      "live.complete": "完成", "live.ready": "准备", "live.transition": "转换",
      "live.waitingFirstScan": "等待第一次刷卡", "live.allComplete": "全部 {0} 站完成",
      "live.movingTo": "前往 {0}",

      "ex.RUN": "跑步", "ex.SKIERG": "滑雪机", "ex.ROWERG": "划船机",
      "ex.SLED_PUSH": "推雪橇", "ex.SLED_PULL": "拉雪橇",
      "ex.BURPEE_BROAD_JUMP": "波比跳远", "ex.FARMERS_CARRY": "农夫行走",
      "ex.SANDBAG_LUNGE": "沙袋箭步蹲", "ex.WALL_BALL": "药球上抛",

      "unit.METER": "米", "unit.KILOMETER": "公里", "unit.REPS": "次",
      "unit.SECOND": "秒", "unit.MINUTE": "分", "unit.CALORIE": "大卡",

      "cat.FOUNDATIONAL": "基础", "cat.ENGINE": "心肺", "cat.POWER": "力量",
      "cat.COMPLETE": "全项", "cat.RACE_SIMULATION": "模拟赛", "cat.CUSTOM": "自定义",

      "blk.SEQUENTIAL": "顺序", "blk.ROUNDS": "回合", "blk.AMRAP": "AMRAP",
      "blk.INTERVAL": "间歇", "blk.ZONE_ROTATION": "分区轮换",

      "stage.PENDING": "待进行", "stage.READY": "下一站", "stage.ACTIVE": "进行中",
      "stage.COMPLETED": "已完成", "stage.SKIPPED": "跳过", "stage.DNF": "未完成",

      "st.DRAFT": "草稿", "st.READY": "准备就绪", "st.RUNNING": "进行中",
      "st.PAUSED": "已暂停", "st.COMPLETED": "已结束", "st.CANCELLED": "已取消",
    },

    "en": {
      "nav.workouts": "Workouts", "nav.builder": "Builder", "nav.class": "Class",
      "nav.live": "Live", "app.title": "HYROX Workout Builder",
      "freshness.none": "no events yet", "freshness.ago": "last event {0} ago",
      "action.cancel": "Cancel", "action.ok": "OK",

      "list.search": "Search workouts", "list.create": "+ Create Workout",
      "list.empty": "No workouts yet.",
      "list.system": "SYSTEM", "list.mine": "MINE",
      "list.view": "View", "list.edit": "Edit", "list.duplicate": "Duplicate",
      "list.use": "Use", "list.delete": "Delete",
      "list.meta": "{0} · {1} steps{2} · v{3}", "list.minutes": " · {0} min",

      "builder.name": "Workout name", "builder.category": "Category",
      "builder.minutes": "Minutes",
      "builder.readonly": "This is a system workout, so it cannot be changed. Duplicate it to make a version you can edit.",
      "builder.addBlock": "+ Add Block", "builder.addExercise": "+ Add Exercise",
      "builder.removeBlock": "Remove block", "builder.rounds": "Rounds",
      "builder.blockName": "Training Block",
      "builder.save": "Save", "builder.saveAs": "Save As…", "builder.use": "Use Template",
      "builder.newName": "New Workout",

      "class.title": "Create Class", "class.workout": "Workout", "class.name": "Class name",
      "class.coach": "Coach", "class.ends": "Ends", "class.minutes": "Minutes",
      "class.save": "Save Draft",
      "class.today": "Today's class",
      "class.todayNote": "Changes here apply to <b>tonight only</b>. The workout you built is not touched.",
      "finish.COACH_DECIDES": "When the coach says",
      "finish.CLASS_DURATION": "After a fixed time",
      "finish.COURSE_COMPLETE": "When the course is finished",
      "cmd.ready": "Mark Ready", "cmd.start": "Start Class", "cmd.pause": "Pause",
      "cmd.resume": "Resume", "cmd.complete": "Complete", "cmd.cancel": "Cancel",

      "ask.device": "Name this device", "ask.deviceDefault": "COACH TABLET",
      "ask.copyName": "Name for the copy", "ask.copySuffix": "{0} (copy)",
      "ask.saveAs": "Save as", "ask.deleteReason": "Why is this workout being deleted?",
      "ask.cancelReason": "Why is this class being cancelled?",

      "toast.duplicated": "Duplicated. The copy is yours to edit.",
      "toast.deleted": "Deleted.", "toast.saved": "Saved as version {0}.",
      "toast.savedNew": "Saved as a new workout.",
      "toast.classCreated": "Class created as a draft. Adjust tonight's targets below, then start.",
      "toast.retargeted": "Tonight only. The saved workout is unchanged.",
      "toast.pickWorkout": "Pick a workout first.",
      "toast.deviceNeeded": "This device needs a name before it can change anything.",

      "err.OPERATOR_REQUIRED": "This device has no name, so it cannot write anything.",
      "err.INVALID_BODY": "The request was not in the expected shape.",
      "err.UNKNOWN_SESSION": "No such class.",
      "err.UNKNOWN_ATHLETE": "That athlete is not on this class's roster.",
      "err.UNKNOWN_EVENT": "No such event.",
      "err.UNKNOWN_TEMPLATE": "No such workout.",
      "err.ILLEGAL_TRANSITION": "That is not something this class can do right now.",
      "err.HAS_INTERPRETED_EVENTS": "This class already has results, so it cannot go back to draft.",
      "err.SESSION_NOT_EDITABLE": "The plan is locked once the class has started.",
      "err.NO_FINISH_RULE": "This class has no finish rule, so it cannot be ended by hand.",
      "err.TAG_ALREADY_BOUND": "That band is already on someone's wrist.",
      "err.ATHLETE_ALREADY_BOUND": "That athlete already has a band; rebind to swap it.",
      "err.NOT_BOUND": "There is no binding to change.",
      "err.REASON_REQUIRED": "This action changes recorded data, so it needs a reason.",
      "err.TEMPLATE_NOT_EDITABLE": "A system workout cannot be edited or deleted; duplicate it first.",
      "err.TEMPLATE_NOT_RUNNABLE": "This workout cannot be run as written.",
      "err.CLASS_IN_PROGRESS": "A class is already in progress; complete or cancel it first.",
      "err.STORAGE_FAILED": "The hub's store rejected the write.",
      "err.unknown": "Something unexpected went wrong.",

      "live.hub": "CENTRAL HUB", "live.course": "COURSE", "live.stations": "STATIONS",
      "live.classElapsed": "CLASS ELAPSED", "live.readersOnline": "READERS ONLINE",
      "live.inClass": "IN CLASS", "live.finished": "FINISHED", "live.exceptions": "EXCEPTIONS",
      "live.connecting": "CONNECTING", "live.noData": "NO DATA", "live.live": "LIVE",
      "live.disconnected": "DISCONNECTED", "live.linkDown": "LINK DOWN",
      "live.noEvents": "NO EVENTS YET", "live.notStarted": "NOT STARTED",
      "live.inStation": "IN STATION", "live.inTransition": "IN TRANSITION",
      "live.moving": "MOVING", "live.complete": "COMPLETE", "live.ready": "READY",
      "live.transition": "TRANSITION",
      "live.waitingFirstScan": "WAITING FOR FIRST SCAN", "live.allComplete": "ALL {0} STATIONS COMPLETE",
      "live.movingTo": "MOVING TO {0}",

      "ex.RUN": "Run", "ex.SKIERG": "SkiErg", "ex.ROWERG": "RowErg",
      "ex.SLED_PUSH": "Sled Push", "ex.SLED_PULL": "Sled Pull",
      "ex.BURPEE_BROAD_JUMP": "Burpee Broad Jump", "ex.FARMERS_CARRY": "Farmers Carry",
      "ex.SANDBAG_LUNGE": "Sandbag Lunge", "ex.WALL_BALL": "Wall Ball",

      "unit.METER": "M", "unit.KILOMETER": "KM", "unit.REPS": "REPS",
      "unit.SECOND": "S", "unit.MINUTE": "MIN", "unit.CALORIE": "CAL",

      "cat.FOUNDATIONAL": "Foundational", "cat.ENGINE": "Engine", "cat.POWER": "Power",
      "cat.COMPLETE": "Complete", "cat.RACE_SIMULATION": "Race Simulation",
      "cat.CUSTOM": "Custom",

      "blk.SEQUENTIAL": "Sequential", "blk.ROUNDS": "Rounds", "blk.AMRAP": "AMRAP",
      "blk.INTERVAL": "Interval", "blk.ZONE_ROTATION": "Zone Rotation",

      "stage.PENDING": "Pending", "stage.READY": "Ready", "stage.ACTIVE": "Active",
      "stage.COMPLETED": "Completed", "stage.SKIPPED": "Skipped", "stage.DNF": "DNF",

      "st.DRAFT": "DRAFT", "st.READY": "READY", "st.RUNNING": "RUNNING",
      "st.PAUSED": "PAUSED", "st.COMPLETED": "COMPLETED", "st.CANCELLED": "CANCELLED",
    },
  };

  const LANGS = [
    { code: "zh-Hant", label: "繁中" },
    { code: "zh-Hans", label: "简中" },
    { code: "en", label: "EN" },
  ];

  const STORAGE_KEY = "hyrox.lang";

  // Per browser, like the device name (ADR 0001 D1) -- the projector machine and a coach's
  // phone are different devices and may legitimately want different languages. `?lang=` wins
  // so the big screen can be pinned by its URL without anyone touching its storage.
  function detect() {
    const fromQuery = new URLSearchParams(global.location.search).get("lang");
    if (fromQuery && DICT[fromQuery]) return fromQuery;
    let stored = null;
    try { stored = localStorage.getItem(STORAGE_KEY); } catch (e) { /* private mode */ }
    if (stored && DICT[stored]) return stored;
    for (const tag of global.navigator.languages || [global.navigator.language || ""]) {
      const lower = tag.toLowerCase();
      if (lower.startsWith("zh")) {
        // Hant unless the tag says otherwise: this is a Taiwanese gym.
        return /hans|\bcn\b|\bsg\b/.test(lower) ? "zh-Hans" : "zh-Hant";
      }
      if (lower.startsWith("en")) return "en";
    }
    return "zh-Hant";
  }

  let lang = detect();

  /// Look up a key, filling {0}, {1}… positionally. Falls back to English and then to the
  /// key itself, so a missing translation shows something a reader can search for rather
  /// than an empty label.
  function t(key, ...args) {
    const value = (DICT[lang] && DICT[lang][key]) ?? DICT.en[key] ?? key;
    return args.length ? value.replace(/\{(\d+)\}/g, (m, i) => args[i] ?? m) : value;
  }

  /// Translate everything already in the document. `data-i18n` sets textContent;
  /// `data-i18n-html` sets innerHTML (for the two strings with a <b> in them);
  /// `data-i18n-placeholder` and `data-i18n-title` do the obvious.
  function apply(root) {
    const scope = root || global.document;
    scope.querySelectorAll("[data-i18n]").forEach((el) => {
      el.textContent = t(el.getAttribute("data-i18n"));
    });
    scope.querySelectorAll("[data-i18n-html]").forEach((el) => {
      el.innerHTML = t(el.getAttribute("data-i18n-html"));
    });
    scope.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
      el.placeholder = t(el.getAttribute("data-i18n-placeholder"));
    });
    global.document.documentElement.lang = lang;
  }

  function set(next) {
    if (!DICT[next]) return;
    lang = next;
    try { localStorage.setItem(STORAGE_KEY, next); } catch (e) { /* private mode */ }
    global.location.reload();
  }

  /// Renders the language switcher into `host`. A row of three, not a <select>: three
  /// options do not need a menu, and a coach on a gym floor should not have to open one.
  function switcher(host) {
    host.innerHTML = "";
    for (const { code, label } of LANGS) {
      const button = global.document.createElement("button");
      button.type = "button";
      button.textContent = label;
      button.className =
        "px-2 py-1 rounded text-xs " +
        (code === lang ? "bg-primary text-on-primary font-bold" : "text-on-surface-variant");
      button.onclick = () => set(code);
      host.appendChild(button);
    }
  }

  global.I18N = { t, apply, set, switcher, langs: LANGS, get lang() { return lang; } };
})(window);
