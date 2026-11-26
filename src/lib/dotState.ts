/** 轻提示前端状态判定：near（Rust proximity 边沿事件）→ 展开「翻译」胶囊，否则半透明小点。
 *  引导气泡不在此判定——guide 由 Rust 保证只发一次，随当次 dot-state 载荷独立渲染 */
export function resolveState(near: boolean): "dot" | "expanded" {
  return near ? "expanded" : "dot";
}
