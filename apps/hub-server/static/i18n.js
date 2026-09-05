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
      "builder.moveUp": "往上移", "builder.moveDown": "往下移", "builder.dupBlock": "複製區塊",

      // --- create class
      "class.title": "建立課程", "class.workout": "課表", "class.name": "課程名稱",
      "class.coach": "教練", "class.ends": "結束方式", "class.minutes": "分鐘",
      "class.mode": "類型", "mode.TRAINING": "團體課程", "mode.COMPETITION": "競賽",
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
      "err.DEMO_UNAVAILABLE": "這台主機沒有示範資料。",
      "err.DEMO_FAILED": "示範資料載入失敗。",
      "err.UNKNOWN_READER": "這支讀取器不在設定裡。",
      "err.UNSUPPORTED_IMAGE": "這個檔案不是 PNG 或 JPG。SVG 不接受——它可能夾帶程式碼。",
      "err.IMAGE_TOO_LARGE": "檔案太大，上限 512 KB。",
      "err.STORAGE_FAILED": "資料庫寫入失敗。",
      "err.PIN_INVALID": "PIN 碼不正確。",
      "err.unknown": "發生未預期的錯誤。",

      // --- live screen
      "live.hub": "HYROX 訓練與競賽應用系統", "live.course": "課表", "live.stations": "站點",
      "live.classElapsed": "已進行", "live.readersOnline": "讀取器上線",
      "live.inClass": "上課人數", "live.finished": "已完成", "live.exceptions": "異常",
      "live.connecting": "連線中", "live.noData": "無資料", "live.live": "連線正常",
      "live.disconnected": "連線中斷", "live.linkDown": "訊號中斷",
      "live.noEvents": "尚無事件", "live.notStarted": "尚未開始",
      "live.inStation": "站內", "live.inTransition": "轉換中", "live.moving": "移動中",
      "live.complete": "完成", "live.ready": "準備", "live.transition": "轉換",
      "live.waitingFirstScan": "等待第一次刷卡", "live.allComplete": "全部 {0} 站完成",
      "live.movingTo": "前往 {0}",

      // --- check-in (ADR 0010)
      "checkin.title": "報到", "checkin.entrants": "參賽者", "checkin.pending": "待綁定腳環",
      "checkin.name": "姓名", "checkin.bib": "號碼布", "checkin.memberId": "會員編號（選填）",
      "checkin.add": "加入名單", "checkin.walkIn": "現場報名", "checkin.member": "會員",
      "checkin.bindTo": "綁定給", "checkin.bound": "已綁定", "checkin.unbound": "未綁定",
      "checkin.noPending": "目前沒有待綁定的腳環。請刷一下腳環。",
      "checkin.noEntrants": "還沒有人報到。",
      "checkin.added": "已加入：{0}", "checkin.boundOk": "已綁定 {0}",
      "checkin.claimed": "追溯認領了 {0} 筆先前的刷卡紀錄。",
      "checkin.pick": "選一位參賽者",
      // --- settings screen (M6)
      "exc.SESSION_NOT_ARMED": "課程尚未開始", "exc.IMPOSSIBLE_TRANSITION": "站點順序不合",
      "exc.ALREADY_FINISHED": "已完賽後又刷卡", "exc.UNKNOWN_READER": "未設定的 RFID 讀取器",
      "exc.ATHLETE_NOT_IN_SESSION": "不在本堂名單",
      "set.title": "系統設定",
      "set.tab.readers": "RFID 讀取器", "set.tab.devices": "裝置", "set.tab.exceptions": "異常", "set.tab.power": "電源",
      "set.tab.screen": "大螢幕", "set.tab.security": "安全鎖",
      "pin.title": "場館安全鎖", "pin.prompt": "請輸入 4 位數 PIN 碼",
      "pin.defaultHint": "預設 PIN 碼為 2018", "pin.invalid": "PIN 碼不正確",
      "pin.change": "修改 PIN 碼", "pin.current": "目前 PIN 碼",
      "pin.new": "新 PIN 碼（4 碼數字）", "pin.confirm": "確認新 PIN 碼",
      "pin.mismatch": "兩次輸入的新 PIN 碼不一致", "pin.updated": "PIN 碼已更新",
      "pin.cancel": "取消", "pin.clear": "清除", "pin.submit": "解鎖",
      "pin.status": "場館 PIN 碼防護：已啟用",
      "pin.desc": "保護系統電源、讀卡機設置與示範資料。大螢幕投影與學員手機端保持免密碼存取。",
      "set.screen": "大螢幕顯示", "set.pageMs": "換頁間隔（秒）",
      "set.pageMsHint": "人數超過 12 位時大螢幕會分頁輪播。場地深、字要看得久一點就調長。3–120 秒。",
      "set.save": "儲存", "set.saved": "已儲存",
      "set.pageSize": "每頁人數", "set.people": "人",
      "set.logo": "場館 logo", "set.logoHint": "PNG 或 JPG，512 KB 以內。會顯示在大螢幕左上角，排在系統名稱前面。",
      "set.logoUpload": "選擇檔案上傳", "set.logoRemove": "移除", "set.logoNone": "尚未上傳",
      "set.logoSaved": "logo 已更新", "set.logoRemoved": "logo 已移除",
      "set.newReaders": "待設定的 RFID 讀取器",
      "set.newReadersHint": "拿手環碰一下 RFID 讀取器，它就會出現在這裡。不用抄 MAC。",
      "set.noNewReaders": "沒有待設定的 RFID 讀取器。",
      "set.reads": "{0} 次刷卡", "set.lastSeen": "{0} 前",
      "set.station": "站點", "set.mode": "模式", "set.assign": "指派",
      "set.addByHand": "手動新增", "set.addHint": "讀取器還沒上牆、或還沒刷過卡時，可以直接輸入裝置 MAC 與讀取器編號。",
      "set.deviceId": "裝置 MAC（12 碼）", "set.readerId": "讀取器編號", "set.add": "新增",
      "set.remove": "移除", "set.removeReason": "移除原因（之後這支讀取器的刷卡會變成異常）",
      "set.removed": "已移除 {0}",
      "set.registered": "已設定的 RFID 讀取器", "set.noRegistered": "還沒有設定任何 RFID 讀取器。",
      "set.device": "裝置", "set.reader": "RFID 讀取器",
      "set.devices": "邊緣裝置", "set.noDevices": "還沒有裝置回報過。",
      "set.boot": "開機序號", "set.pending": "待送事件", "set.capacity": "journal 容量",
      "set.exceptions": "待處理異常", "set.noExceptions": "沒有待處理的異常。",
      "set.void": "作廢", "set.voidReason": "作廢原因",
      "set.accept": "確認無誤", "set.acceptReason": "確認無誤（可不填原因）",
      "set.confirm": "確定", "set.cancel": "取消",
      "ask.typeYourOwn": "自行輸入",
      "exc.chip.missed": "忘記感應", "exc.chip.wrongStation": "走錯站點",
      "exc.chip.duplicate": "感應重複", "exc.chip.newBand": "更換手環",
      "exc.chip.coachApproved": "教練口頭核准",
      "set.power": "電源", "set.poweroff": "關機", "set.reboot": "重新開機", "set.restart": "重啟主機服務",
      "set.powerHint": "課程進行中不能關機或重開機器；重啟服務隨時可以，它會自己接回原本的課。",
      "set.powerReason": "原因",
      "set.powerAsked": "已送出：{0}",
      "set.tab.demo": "示範資料", "set.demo": "示範資料",
      "set.demoHint": "載入一整場示範課程：課表、12 位選手、讀取器與腳環，並開始模擬刷卡。這是給整合測試用的假資料，不要在真的課程上按。",
      "set.demoLoad": "載入示範資料", "set.demoClear": "停止模擬刷卡",
      "set.demoLoaded": "示範資料已載入，模擬刷卡開始了", "set.demoCleared": "模擬刷卡已停止",
      "set.assigned": "已指派 {0}",
      "mode.ENTRY": "進站", "mode.EXIT": "出站", "mode.TOGGLE": "進出共用",
      "mode.CHECKPOINT": "檢查點", "mode.PASSAGE": "通過",
      "checkin.findCode": "報名編號",
      "checkin.findCodeHint": "掃描選手的 QR，或請他念出六碼",
      "checkin.scan": "用相機掃碼",
      "checkin.scanStop": "停止掃描",
      "checkin.scanUnavailable": "這台裝置的瀏覽器不支援相機掃碼，請用掃描槍或手動輸入六碼。",
      "checkin.scanInsecure": "相機掃碼需要 HTTPS，請用掃描槍或手動輸入六碼。",
      "checkin.codeNotFound": "找不到編號 {0}。",
      // --- self sign-up (ADR 0011)
      "signup.title": "賽事報名",
      "signup.lead": "填名字就好。報名完成後會給你一組六碼編號和 QR，用來領腳環、查成績。",
      "signup.name": "姓名",
      "signup.namePlaceholder": "請輸入姓名",
      "signup.submit": "送出報名",
      "signup.done": "報名完成",
      "signup.yourCode": "你的編號",
      "signup.bib": "號碼布",
      "signup.keep": "請把這個畫面留著或截圖。領腳環和查成績都用它。",
      "signup.showAtDesk": "到報到處出示這個 QR，工作人員會幫你配對腳環。",
      "signup.reopen": "已經報過名？輸入編號查詢",
      "signup.lookup": "查詢",
      "signup.status": "目前狀態",
      "signup.result": "成績",
      "signup.notStarted": "尚未出發",
      "signup.racing": "比賽中",
      "signup.finished": "完賽",
      "signup.place": "名次 {0}",
      "signup.noPlace": "此場次不計名次",
      "signup.stations": "完成 {0} / {1} 站",
      // --- leaderboard / results
      "board.title": "排行榜", "board.place": "名次", "board.athlete": "選手",
      "board.time": "成績", "board.progress": "進度", "board.stations": "站點",
      "board.racing": "比賽中", "board.finished": "完賽", "board.notStarted": "未出發",
      "board.unranked": "此課程不計名次 —— 依號碼布排序",
      "board.unrankedWhy": "時間到就結束的課程，每個人做的份量不同，用時間排名並不誠實。",
      "board.empty": "還沒有人在場上。",
      "result.title": "成績", "result.splits": "分段", "result.station": "站點",
      "result.work": "站內", "result.transition": "轉換", "result.total": "總計",
      "result.dnf": "未完賽", "result.noResults": "這堂課還沒有成績。",
      "result.walkIn": "現場報名",
      "share.btn": "分享戰績", "share.title": "官方戰績卡", "share.download": "下載圖片",
      "share.native": "分享至社群", "share.copied": "已複製到剪貼簿", "share.generating": "生成戰績卡中...",
      "net.reconnecting": "連線中斷，正在重新連線...", "net.restored": "連線已恢復",
      "dev.signalStrong": "訊號強", "dev.signalGood": "訊號良好", "dev.signalFair": "訊號普通",
      "dev.signalWeak": "訊號微弱", "dev.signalOffline": "離線",
      "live.podiumTitle": "完賽榮譽榜", "live.switchPodium": "切換排行榜 (L)", "live.switchGrid": "返回計時牆 (L)",
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
      "builder.moveUp": "上移", "builder.moveDown": "下移", "builder.dupBlock": "复制区块",

      "class.title": "创建课程", "class.workout": "课表", "class.name": "课程名称",
      "class.coach": "教练", "class.ends": "结束方式", "class.minutes": "分钟",
      "class.mode": "类型", "mode.TRAINING": "团体课程", "mode.COMPETITION": "竞赛",
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
      "err.DEMO_UNAVAILABLE": "这台主机没有示范数据。",
      "err.DEMO_FAILED": "示范数据载入失败。",
      "err.UNKNOWN_READER": "这支读取器不在设置里。",
      "err.UNSUPPORTED_IMAGE": "这个文件不是 PNG 或 JPG。SVG 不接受——它可能夹带代码。",
      "err.IMAGE_TOO_LARGE": "文件太大，上限 512 KB。",
      "err.STORAGE_FAILED": "数据库写入失败。",
      "err.PIN_INVALID": "PIN 码不正确。",
      "err.unknown": "发生未预期的错误。",

      "live.hub": "HYROX 训练与竞赛应用系统", "live.course": "课表", "live.stations": "站点",
      "live.classElapsed": "已进行", "live.readersOnline": "读取器在线",
      "live.inClass": "上课人数", "live.finished": "已完成", "live.exceptions": "异常",
      "live.connecting": "连接中", "live.noData": "无数据", "live.live": "连接正常",
      "live.disconnected": "连接中断", "live.linkDown": "信号中断",
      "live.noEvents": "尚无事件", "live.notStarted": "尚未开始",
      "live.inStation": "站内", "live.inTransition": "转换中", "live.moving": "移动中",
      "live.complete": "完成", "live.ready": "准备", "live.transition": "转换",
      "live.waitingFirstScan": "等待第一次刷卡", "live.allComplete": "全部 {0} 站完成",
      "live.movingTo": "前往 {0}",

      "checkin.title": "报到", "checkin.entrants": "参赛者", "checkin.pending": "待绑定脚环",
      "checkin.name": "姓名", "checkin.bib": "号码布", "checkin.memberId": "会员编号（选填）",
      "checkin.add": "加入名单", "checkin.walkIn": "现场报名", "checkin.member": "会员",
      "checkin.bindTo": "绑定给", "checkin.bound": "已绑定", "checkin.unbound": "未绑定",
      "checkin.noPending": "目前没有待绑定的脚环。请刷一下脚环。",
      "checkin.noEntrants": "还没有人报到。",
      "checkin.added": "已加入：{0}", "checkin.boundOk": "已绑定 {0}",
      "checkin.claimed": "追溯认领了 {0} 条先前的刷卡记录。",
      "checkin.pick": "选一位参赛者",
      // --- settings screen (M6)
      "exc.SESSION_NOT_ARMED": "课程尚未开始", "exc.IMPOSSIBLE_TRANSITION": "站点顺序不合",
      "exc.ALREADY_FINISHED": "已完赛后又刷卡", "exc.UNKNOWN_READER": "未设置的 RFID 读取器",
      "exc.ATHLETE_NOT_IN_SESSION": "不在本堂名单",
      "set.title": "系统设置",
      "set.tab.readers": "RFID 读取器", "set.tab.devices": "设备", "set.tab.exceptions": "异常", "set.tab.power": "电源",
      "set.tab.screen": "大屏幕", "set.tab.security": "安全锁",
      "pin.title": "场馆安全锁", "pin.prompt": "请输入 4 位数 PIN 码",
      "pin.defaultHint": "默认 PIN 码为 2018", "pin.invalid": "PIN 码不正确",
      "pin.change": "修改 PIN 码", "pin.current": "当前 PIN 码",
      "pin.new": "新 PIN 码（4 位数字）", "pin.confirm": "确认新 PIN 码",
      "pin.mismatch": "两次输入的新 PIN 码不一致", "pin.updated": "PIN 码已更新",
      "pin.cancel": "取消", "pin.clear": "清除", "pin.submit": "解锁",
      "pin.status": "场馆 PIN 码防护：已启用",
      "pin.desc": "保护系统电源、读卡器配置与示范数据。大屏幕投影与学员手机端保持免密码访问。",
      "set.screen": "大屏幕显示", "set.pageMs": "换页间隔（秒）",
      "set.pageMsHint": "人数超过 12 位时大屏幕会分页轮播。场地深、字要看久一点就调长。3–120 秒。",
      "set.save": "保存", "set.saved": "已保存",
      "set.pageSize": "每页人数", "set.people": "人",
      "set.logo": "场馆 logo", "set.logoHint": "PNG 或 JPG，512 KB 以内。会显示在大屏幕左上角，排在系统名称前面。",
      "set.logoUpload": "选择文件上传", "set.logoRemove": "移除", "set.logoNone": "尚未上传",
      "set.logoSaved": "logo 已更新", "set.logoRemoved": "logo 已移除",
      "set.newReaders": "待设置的 RFID 读取器",
      "set.newReadersHint": "拿手环碰一下 RFID 读取器，它就会出现在这里。不用抄 MAC。",
      "set.noNewReaders": "没有待设置的 RFID 读取器。",
      "set.reads": "{0} 次刷卡", "set.lastSeen": "{0} 前",
      "set.station": "站点", "set.mode": "模式", "set.assign": "指派",
      "set.addByHand": "手动新增", "set.addHint": "读取器还没上墙、或还没刷过卡时，可以直接输入设备 MAC 与读取器编号。",
      "set.deviceId": "设备 MAC（12 码）", "set.readerId": "读取器编号", "set.add": "新增",
      "set.remove": "移除", "set.removeReason": "移除原因（之后这支读取器的刷卡会变成异常）",
      "set.removed": "已移除 {0}",
      "set.registered": "已设置的 RFID 读取器", "set.noRegistered": "还没有设置任何 RFID 读取器。",
      "set.device": "设备", "set.reader": "RFID 读取器",
      "set.devices": "边缘设备", "set.noDevices": "还没有设备回报过。",
      "set.boot": "开机序号", "set.pending": "待发送事件", "set.capacity": "journal 容量",
      "set.exceptions": "待处理异常", "set.noExceptions": "没有待处理的异常。",
      "set.void": "作废", "set.voidReason": "作废原因",
      "set.accept": "确认无误", "set.acceptReason": "确认无误（可不填原因）",
      "set.confirm": "确定", "set.cancel": "取消",
      "ask.typeYourOwn": "自行输入",
      "exc.chip.missed": "忘记感应", "exc.chip.wrongStation": "走错站点",
      "exc.chip.duplicate": "感应重复", "exc.chip.newBand": "更换手环",
      "exc.chip.coachApproved": "教练口头核准",
      "set.power": "电源", "set.poweroff": "关机", "set.reboot": "重新启动", "set.restart": "重启主机服务",
      "set.powerHint": "课程进行中不能关机或重启机器；重启服务随时可以，它会自己接回原本的课。",
      "set.powerReason": "原因",
      "set.powerAsked": "已送出：{0}",
      "set.tab.demo": "示范数据", "set.demo": "示范数据",
      "set.demoHint": "载入一整场示范课程：课表、12 位选手、读取器与脚环，并开始模拟刷卡。这是给整合测试用的假数据，不要在真的课程上按。",
      "set.demoLoad": "载入示范数据", "set.demoClear": "停止模拟刷卡",
      "set.demoLoaded": "示范数据已载入，模拟刷卡开始了", "set.demoCleared": "模拟刷卡已停止",
      "set.assigned": "已指派 {0}",
      "mode.ENTRY": "进站", "mode.EXIT": "出站", "mode.TOGGLE": "进出共用",
      "mode.CHECKPOINT": "检查点", "mode.PASSAGE": "通过",
      "checkin.findCode": "报名编号",
      "checkin.findCodeHint": "扫描选手的 QR，或请他念出六码",
      "checkin.scan": "用相机扫码",
      "checkin.scanStop": "停止扫描",
      "checkin.scanUnavailable": "这台设备的浏览器不支持相机扫码，请用扫描枪或手动输入六码。",
      "checkin.scanInsecure": "相机扫码需要 HTTPS，请用扫描枪或手动输入六码。",
      "checkin.codeNotFound": "找不到编号 {0}。",
      // --- self sign-up (ADR 0011)
      "signup.title": "赛事报名",
      "signup.lead": "填名字就好。报名完成后会给你一组六码编号和 QR，用来领脚环、查成绩。",
      "signup.name": "姓名",
      "signup.namePlaceholder": "请输入姓名",
      "signup.submit": "送出报名",
      "signup.done": "报名完成",
      "signup.yourCode": "你的编号",
      "signup.bib": "号码布",
      "signup.keep": "请把这个画面留着或截图。领脚环和查成绩都用它。",
      "signup.showAtDesk": "到报到处出示这个 QR，工作人员会帮你配对脚环。",
      "signup.reopen": "已经报过名？输入编号查询",
      "signup.lookup": "查询",
      "signup.status": "当前状态",
      "signup.result": "成绩",
      "signup.notStarted": "尚未出发",
      "signup.racing": "比赛中",
      "signup.finished": "完赛",
      "signup.place": "名次 {0}",
      "signup.noPlace": "此场次不计名次",
      "signup.stations": "完成 {0} / {1} 站",
      "board.title": "排行榜", "board.place": "名次", "board.athlete": "选手",
      "board.time": "成绩", "board.progress": "进度", "board.stations": "站点",
      "board.racing": "比赛中", "board.finished": "完赛", "board.notStarted": "未出发",
      "board.unranked": "此课程不计名次 —— 按号码布排序",
      "board.unrankedWhy": "时间到就结束的课程，每个人做的份量不同，用时间排名并不诚实。",
      "board.empty": "还没有人在场上。",
      "result.title": "成绩", "result.splits": "分段", "result.station": "站点",
      "result.work": "站内", "result.transition": "转换", "result.total": "总计",
      "result.dnf": "未完赛", "result.noResults": "这堂课还没有成绩。",
      "result.walkIn": "现场报名",
      "share.btn": "分享战绩", "share.title": "官方战绩卡", "share.download": "下载图片",
      "share.native": "分享至社群", "share.copied": "已复制到剪贴板", "share.generating": "生成战绩卡中...",
      "net.reconnecting": "连接中断，正在重新连接...", "net.restored": "连接已恢复",
      "dev.signalStrong": "信号强", "dev.signalGood": "信号良好", "dev.signalFair": "信号普通",
      "dev.signalWeak": "信号微弱", "dev.signalOffline": "离线",
      "live.podiumTitle": "完赛荣誉榜", "live.switchPodium": "切换排行榜 (L)", "live.switchGrid": "返回计时墙 (L)",
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
      "builder.moveUp": "Move Up", "builder.moveDown": "Move Down", "builder.dupBlock": "Duplicate Block",

      "class.title": "Create Class", "class.workout": "Workout", "class.name": "Class name",
      "class.coach": "Coach", "class.ends": "Ends", "class.minutes": "Minutes",
      "class.mode": "Type", "mode.TRAINING": "Group class", "mode.COMPETITION": "Competition",
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
      "err.DEMO_UNAVAILABLE": "This hub does not carry demo data.",
      "err.DEMO_FAILED": "Demo data could not be loaded.",
      "err.UNKNOWN_READER": "No reader is registered under that id.",
      "err.UNSUPPORTED_IMAGE": "That file is not a PNG or JPG. SVG is not accepted: it can carry script.",
      "err.IMAGE_TOO_LARGE": "That file is too large; the limit is 512 KB.",
      "err.STORAGE_FAILED": "The hub's store rejected the write.",
      "err.PIN_INVALID": "Incorrect PIN.",
      "err.unknown": "Something unexpected went wrong.",

      "live.hub": "HYROX TRAINING & COMPETITION SYSTEM", "live.course": "COURSE", "live.stations": "STATIONS",
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

      "checkin.title": "Check-in", "checkin.entrants": "Entrants", "checkin.pending": "Bands waiting",
      "checkin.name": "Name", "checkin.bib": "Bib", "checkin.memberId": "Member ID (optional)",
      "checkin.add": "Add to roster", "checkin.walkIn": "Walk-in", "checkin.member": "Member",
      "checkin.bindTo": "Bind to", "checkin.bound": "Band on", "checkin.unbound": "No band",
      "checkin.noPending": "No bands waiting. Tap one on a reader.",
      "checkin.noEntrants": "Nobody has checked in yet.",
      "checkin.added": "Added {0}", "checkin.boundOk": "Band bound to {0}",
      "checkin.claimed": "Claimed {0} earlier reads.",
      "checkin.pick": "Pick an entrant",
      // --- settings screen (M6)
      "exc.SESSION_NOT_ARMED": "Class not started", "exc.IMPOSSIBLE_TRANSITION": "Out of order",
      "exc.ALREADY_FINISHED": "Read after finishing", "exc.UNKNOWN_READER": "Reader not configured",
      "exc.ATHLETE_NOT_IN_SESSION": "Not on this roster",
      "set.title": "Settings",
      "set.tab.readers": "Readers", "set.tab.devices": "Devices", "set.tab.exceptions": "Exceptions", "set.tab.power": "Power",
      "set.tab.screen": "Screen", "set.tab.security": "Security",
      "pin.title": "Venue Security Lock", "pin.prompt": "Enter 4-digit PIN",
      "pin.defaultHint": "Default PIN is 2018", "pin.invalid": "Incorrect PIN",
      "pin.change": "Change PIN", "pin.current": "Current PIN",
      "pin.new": "New PIN (4 digits)", "pin.confirm": "Confirm New PIN",
      "pin.mismatch": "PINs do not match", "pin.updated": "PIN updated",
      "pin.cancel": "Cancel", "pin.clear": "Clear", "pin.submit": "Unlock",
      "pin.status": "Venue PIN Protection: Active",
      "pin.desc": "Protects system power, reader assignments, and demo data. The live screen and self signup remain password-free.",
      "set.screen": "Live screen", "set.pageMs": "Seconds per page",
      "set.pageMsHint": "Above twelve people the live screen rotates pages. A long room reads slower than a studio. 3-120 seconds.",
      "set.save": "Save", "set.saved": "Saved",
      "set.pageSize": "People per page", "set.people": "people",
      "set.logo": "Venue logo", "set.logoHint": "PNG or JPG, up to 512 KB. Shown top left on the live screen, ahead of the system name.",
      "set.logoUpload": "Choose a file", "set.logoRemove": "Remove", "set.logoNone": "None uploaded",
      "set.logoSaved": "Logo updated", "set.logoRemoved": "Logo removed",
      "set.newReaders": "Readers waiting to be set up",
      "set.newReadersHint": "Tap an antenna with any band and it appears here. No MAC addresses to copy.",
      "set.noNewReaders": "No readers waiting.",
      "set.reads": "{0} reads", "set.lastSeen": "{0} ago",
      "set.station": "Station", "set.mode": "Mode", "set.assign": "Assign",
      "set.addByHand": "Add by hand", "set.addHint": "For a reader that is not on the wall yet, or has not been tapped: type the device MAC and reader id.",
      "set.deviceId": "Device MAC (12 hex)", "set.readerId": "Reader id", "set.add": "Add",
      "set.remove": "Remove", "set.removeReason": "Why (reads from it will become exceptions)",
      "set.removed": "Removed {0}",
      "set.registered": "Configured readers", "set.noRegistered": "No readers configured yet.",
      "set.device": "Device", "set.reader": "Reader",
      "set.devices": "Edge devices", "set.noDevices": "No device has reported yet.",
      "set.boot": "Boot", "set.pending": "Pending events", "set.capacity": "Journal capacity",
      "set.exceptions": "Exceptions", "set.noExceptions": "Nothing waiting.",
      "set.void": "Void", "set.voidReason": "Reason for voiding",
      "set.accept": "Accept as is", "set.acceptReason": "Accept as is (reason optional)",
      "set.confirm": "Confirm", "set.cancel": "Cancel",
      "ask.typeYourOwn": "Type your own",
      "exc.chip.missed": "Missed the tap", "exc.chip.wrongStation": "Wrong station",
      "exc.chip.duplicate": "Duplicate read", "exc.chip.newBand": "Band swapped",
      "exc.chip.coachApproved": "Coach approved",
      "set.power": "Power", "set.poweroff": "Shut down", "set.reboot": "Restart machine", "set.restart": "Restart hub service",
      "set.powerHint": "The machine cannot be stopped while a class is on the floor. Restarting the service is always safe: it rejoins the class by itself.",
      "set.powerReason": "Reason",
      "set.powerAsked": "Sent: {0}",
      "set.tab.demo": "Demo data", "set.demo": "Demo data",
      "set.demoHint": "Loads a whole demo class -- course, 12 athletes, readers and bands -- and starts emulated reads. Test-machine data: do not press this during a real class.",
      "set.demoLoad": "Load demo data", "set.demoClear": "Stop emulated reads",
      "set.demoLoaded": "Demo data loaded; emulated reads have started", "set.demoCleared": "Emulated reads stopped",
      "set.assigned": "Assigned {0}",
      "mode.ENTRY": "Entry", "mode.EXIT": "Exit", "mode.TOGGLE": "Toggle",
      "mode.CHECKPOINT": "Checkpoint", "mode.PASSAGE": "Passage",
      "checkin.findCode": "Entry code",
      "checkin.findCodeHint": "Scan the entrant's QR, or ask them to read out the six characters",
      "checkin.scan": "Scan with camera",
      "checkin.scanStop": "Stop scanning",
      "checkin.scanUnavailable": "This browser cannot scan with the camera. Use a scanner or type the six characters.",
      "checkin.scanInsecure": "Camera scanning needs HTTPS. Use a scanner or type the six characters.",
      "checkin.codeNotFound": "No entrant with code {0}.",
      // --- self sign-up (ADR 0011)
      "signup.title": "Race entry",
      "signup.lead": "Your name is all we need. You get a six-character code and a QR to collect your band and look up your result.",
      "signup.name": "Name",
      "signup.namePlaceholder": "Your name",
      "signup.submit": "Enter",
      "signup.done": "You are entered",
      "signup.yourCode": "Your code",
      "signup.bib": "Bib",
      "signup.keep": "Keep this screen or take a screenshot. It is how you collect your band and find your result.",
      "signup.showAtDesk": "Show this QR at the desk and a helper will pair your band.",
      "signup.reopen": "Already entered? Look up your code",
      "signup.lookup": "Look up",
      "signup.status": "Status",
      "signup.result": "Result",
      "signup.notStarted": "Not started",
      "signup.racing": "Racing",
      "signup.finished": "Finished",
      "signup.place": "Place {0}",
      "signup.noPlace": "This class has no places",
      "signup.stations": "{0} of {1} stations",
      "board.title": "Leaderboard", "board.place": "Place", "board.athlete": "Athlete",
      "board.time": "Time", "board.progress": "Progress", "board.stations": "Stations",
      "board.racing": "Racing", "board.finished": "Finished", "board.notStarted": "Not started",
      "board.unranked": "This session is not ranked \u2014 shown in bib order",
      "board.unrankedWhy": "A class that ends on the clock stops everyone having done different amounts of work, so ordering by time would not be honest.",
      "board.empty": "Nobody on the floor yet.",
      "result.title": "Results", "result.splits": "Splits", "result.station": "Station",
      "result.work": "Work", "result.transition": "Transition", "result.total": "Total",
      "result.dnf": "DNF", "result.noResults": "This class has no results yet.",
      "result.walkIn": "Walk-in",
      "share.btn": "Share Story", "share.title": "Official Workout Card", "share.download": "Download PNG",
      "share.native": "Share", "share.copied": "Copied to clipboard", "share.generating": "Generating card...",
      "net.reconnecting": "Disconnected, reconnecting...", "net.restored": "Connected",
      "dev.signalStrong": "Strong", "dev.signalGood": "Good", "dev.signalFair": "Fair",
      "dev.signalWeak": "Weak", "dev.signalOffline": "Offline",
      "live.podiumTitle": "Podium & Results", "live.switchPodium": "Switch to Leaderboard (L)", "live.switchGrid": "Back to Live Wall (L)",
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
