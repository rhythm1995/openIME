// 品牌声波 mark：5 条对称竖条，高度比 180/300/420/300/180（与 logo 几何一致）。
export default function WaveMark({ size = 22 }: { size?: number }) {
  const heights = [14, 20, 28, 20, 14];
  return (
    <svg
      width={size}
      height={(size * 32) / 40}
      viewBox="0 0 40 32"
      fill="none"
      aria-hidden="true"
      style={{ display: "block", color: "currentColor" }}
    >
      {heights.map((h, i) => (
        <rect
          key={i}
          x={i * 9}
          y={(32 - h) / 2}
          width="4"
          height={h}
          rx="2"
          fill="currentColor"
        />
      ))}
    </svg>
  );
}
