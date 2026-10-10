/** 划词事件载荷（Rust → 前端 selection 事件） */
export interface Selection {
  text: string;
  x: number;
  y: number;
}

export interface BaiduConfig {
  appid: string;
  secret: string;
}

/** 配置文件结构（与后端 AppConfig 对应，snake_case） */
export interface AppConfig {
  source: string;
  hotkey: string;
  screenshot_hotkey: string;
  ocr_lang: string;
  baidu: BaiduConfig;
}

export interface UpdateStatus {
  available: boolean;
  current_version: string;
  new_version: string | null;
}

export interface WordForm {
  name: string;
  value: string;
}

export interface SynoGroup {
  pos: string;
  tran: string;
  words: string[];
}

export interface WordInfo {
  word: string;
  source: string;
  usphone: string | null;
  ukphone: string | null;
  senses: string[];
  word_forms: WordForm[];
  synos: SynoGroup[];
  exam_types: string[];
}

/** 截图翻译结果卡片载荷（screenshot-result 事件） */
export interface ShotResult {
  ok: boolean;
  text: string | null;
  info: WordInfo | null;
  error: string | null;
  /** 框选区物理坐标（虚拟屏坐标系），用于卡片定位 */
  sel: { x: number; y: number; w: number; h: number };
  /** 虚拟屏物理边界，用于卡片位置錨制 */
  virtual: { x: number; y: number; width: number; height: number };
}
