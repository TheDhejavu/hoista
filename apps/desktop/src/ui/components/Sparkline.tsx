/** Seven little grey bars, one per day, for tables. */
export function BarSparkline({ values, height = 18 }: { values: number[]; height?: number }) {
  const max = Math.max(...values, 1);
  const barWidth = 4;
  const gap = 2;
  const width = values.length * (barWidth + gap) - gap;

  return (
    <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} aria-hidden="true">
      {values.map((value, index) => {
        const barHeight = Math.max(3, (value / max) * height);
        return (
          <rect
            key={index}
            x={index * (barWidth + gap)}
            y={height - barHeight}
            width={barWidth}
            height={barHeight}
            rx="1.5"
            fill="var(--spark)"
          />
        );
      })}
    </svg>
  );
}
