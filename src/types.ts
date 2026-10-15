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

export interface TencentConfig {
  secret_id: string;
  secret_key: string;
}

/** 配置文件结构（与后端 AppConfig 对应，snake_case） */
export interface AppConfig {
  source: string;
  sources: string[];
  hotkey: string;
  screenshot_hotkey: string;
  ocr_lang: string;
  input_hotkey: string;
  /** 内置默认源（有道词典/必应翻译）是否启用 */
  builtin_sources: boolean;
  /** 设置窗口上次停留的一级菜单，重开时恢复 */
  settings_section?: string;
  baidu: BaiduConfig;
  tencent: TencentConfig;
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
  /** 框选区物理坐标（虚拟屏坐标系） */
  sel: { x: number; y: number; w: number; h: number };
  /** 虚拟屏物理边界，用于卡片位置錨制 */
  virtual: { x: number; y: number; width: number; height: number };
  /** 松手瞬间鼠标位置（物理像素），卡片弹到鼠标旁边 */
  mouse: { x: number; y: number };
  /** 光标所在屏的工作区（排除任务栏，物理像素）；width/height 为 0 时表示未获取到 */
  work: { x: number; y: number; width: number; height: number };
}

/** 逐源对比结果（translate_all 命令） */
export interface SourceResult {
  source: string;
  label: string;
  ok: boolean;
  senses: string[];
  error: string | null;
}
