// 有道发音播放（模块级共享 Audio：每次 new 会各留一份解码器等 GC，高频点发音会堆内存）
let audio: HTMLAudioElement | null = null;

/**
 * 播放有道词典发音，只按单词字符串取音，与当前翻译源无关。
 * type=2 美音 / type=1 英音 / type=0 通用 TTS 兜底
 */
export function playVoice(word: string, type: 1 | 2 | 0) {
  const url = (t: number) =>
    `https://dict.youdao.com/dictvoice?audio=${encodeURIComponent(word)}&type=${t}`;
  audio ??= new Audio();
  audio.src = url(type);
  // 个别词条缺指定音色录音（接口返回 500），退回通用发音
  audio.onerror = () => {
    if (!audio) return;
    audio.onerror = null;
    audio.src = url(0);
    audio.play();
  };
  audio.play();
}
