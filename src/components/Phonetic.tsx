import { VolumeIcon } from "./icons";
import { playVoice } from "../lib/voice";

/** 有道音标 + 发音（volume-2 图标 美 / 英）：Popup 划词卡 / InputPage 输入翻译共用 */
export default function Phonetic({
  word,
  usphone,
  ukphone,
}: {
  word: string;
  usphone: string | null;
  ukphone: string | null;
}) {
  return (
    <>
      {usphone && (
        <span className="phonetic" title="播放美音" onClick={() => playVoice(word, 2)}>
          <VolumeIcon /> 美 {usphone}
        </span>
      )}
      {ukphone && (
        <span className="phonetic" title="播放英音" onClick={() => playVoice(word, 1)}>
          <VolumeIcon /> 英 {ukphone}
        </span>
      )}
    </>
  );
}
