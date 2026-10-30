import { useRef } from "react";

/** 有道音标 + 发音（🔊 美 / 🔊 英）：Popup 划词卡 / InputPage 输入翻译共用 */
export default function Phonetic({
  word,
  usphone,
  ukphone,
}: {
  word: string;
  usphone: string | null;
  ukphone: string | null;
}) {
  // 播放有道词典发音，只按单词字符串取音，与当前翻译源无关。
  // 复用同一个 Audio：每次 new 会各留一份解码器等 GC，高频点发音会堆内存
  // type=2 美音 / type=1 英音 / type=0 通用 TTS 兜底
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const playVoice = (type: 1 | 2) => {
    const url = (t: number) =>
      `https://dict.youdao.com/dictvoice?audio=${encodeURIComponent(word)}&type=${t}`;
    const audio = audioRef.current ?? new Audio();
    audioRef.current = audio;
    audio.src = url(type);
    // 个别词条缺指定音色录音（接口返回 500），退回通用发音
    audio.onerror = () => {
      audio.onerror = null;
      audio.src = url(0);
      audio.play();
    };
    audio.play();
  };

  return (
    <>
      {usphone && (
        <span className="phonetic" title="播放美音" onClick={() => playVoice(2)}>
          🔊 美 {usphone}
        </span>
      )}
      {ukphone && (
        <span className="phonetic" title="播放英音" onClick={() => playVoice(1)}>
          🔊 英 {ukphone}
        </span>
      )}
    </>
  );
}
