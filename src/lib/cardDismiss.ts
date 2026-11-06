/** 卡片收起判定的纯谓词（设计文档 §5.1 形式化）。事实由轮询收集，判定本身零副作用、可单测。 */

/** 轮询收集到的一次收起判定事实 */
export interface DismissFacts {
  /** 鼠标悬在卡片上：永不消失 */
  cursorInCard: boolean;
  /** 本场卡片真正拿到过焦点 */
  everFocused: boolean;
  /** 当前是否有焦点 */
  focused: boolean;
  /** show 之后点过卡片外（仅无焦点窗口的模式二需要） */
  externalClick: boolean;
}

/**
 * 收起判定：
 *   dismiss? ≡ ¬cursorInCard
 *            ∧ ( everFocused ∧ ¬focused          ← 模式一：可聚焦窗口，blur 语义
 *              ∨ ¬everFocused ∧ externalClick )  ← 模式二：无焦点窗口（mac makeKey 失灵），
 *                                                  全局鼠标命中语义
 *
 * 注意：没有 visible 项。曾试过「窗口藏起（截图会话）时不收」，
 * 实测 isVisible() 在卡片显示期间偶发返回 false，轮询会被停死导致卡片永不收起——回归原版语义。
 */
export function shouldDismiss(f: DismissFacts): boolean {
  if (f.cursorInCard) return false;
  return f.everFocused ? !f.focused : f.externalClick;
}

/** 点坐标是否落在窗口矩形内（物理像素；outerPosition/outerSize） */
export function pointInRect(
  x: number,
  y: number,
  pos: { x: number; y: number },
  size: { width: number; height: number },
): boolean {
  return x >= pos.x && x < pos.x + size.width && y >= pos.y && y < pos.y + size.height;
}

/**
 * 模式二的「show 之后点过卡片外」：
 * - 入场豁免：只认 show 之后的全局按下，选词手势/图标点击那一下不算
 * - 点在自己窗口矩形内不算外部（透明阴影边距的收起交给 DOM onDocMouseDown 判）
 */
export function externalClickAfterShow(
  md: [number, number, number] | null,
  shownAt: number,
  clickedInsideWindow: boolean,
): boolean {
  if (!md || md[2] <= shownAt) return false;
  return !clickedInsideWindow;
}
