import type { HealthLevel, StageId } from './stage';

interface LinkGraphicProps {
  stage: StageId;
  fps: number;
  label: string;
  /** Colours the moving data: signal when smooth, amber or red when the link is struggling. */
  tone?: HealthLevel;
}

/** The cable's route, shared by the base line and the flowing overlay. */
const CABLE = 'M132 112 C 240 112, 262 152, 346 152 S 470 112, 558 112';

/**
 * Phone, cable and computer.
 *
 * The cable is the only moving thing on the dashboard, and it moves only when
 * data does: dashes travel faster as the frame rate rises, hold still while
 * the link is up but idle, and the cable is simply not connected before a
 * phone is plugged in.
 */
const FLOW_COLOUR: Record<HealthLevel, string> = {
  good: 'stroke-signal',
  warn: 'stroke-warn',
  bad: 'stroke-bad',
};

export function LinkGraphic({ stage, fps, label, tone = 'good' }: LinkGraphicProps) {
  const noPhone = stage === 'no-phone';
  const linked = stage === 'ready' || stage === 'waiting-for-phone' || stage === 'mirroring';
  const flowing = stage === 'waiting-for-phone' || stage === 'mirroring';

  // 60 fps crosses the cable in about 0.8 s; 120 in 0.4 s; waiting is a slow crawl.
  const flowSeconds =
    stage === 'mirroring' ? Math.min(2, Math.max(0.3, 48 / Math.max(fps, 24))) : 2.4;

  return (
    <svg
      viewBox="0 0 760 200"
      role="img"
      aria-label={label}
      className="mx-auto h-auto max-h-[150px] w-full"
    >
      {/* Cable body */}
      <path
        d={CABLE}
        fill="none"
        strokeWidth={linked ? 5 : 3}
        strokeLinecap="round"
        strokeDasharray={noPhone ? '1 10' : undefined}
        className={linked ? 'stroke-signal/25' : 'stroke-line'}
      />

      {/* Data travelling along it */}
      {flowing && (
        <path
          d={CABLE}
          fill="none"
          strokeWidth="5"
          strokeLinecap="round"
          className={`cable-flow ${FLOW_COLOUR[tone]}`}
          style={{ ['--flow-duration' as string]: `${flowSeconds}s` }}
        />
      )}

      {/* Phone */}
      <g className={noPhone ? 'opacity-45' : undefined}>
        <rect
          x="60"
          y="40"
          width="72"
          height="140"
          rx="16"
          strokeWidth="3"
          className={`fill-panel ${stage === 'phone-found' ? 'stroke-signal' : 'stroke-ink'}`}
        />
        <rect
          x="69"
          y="54"
          width="54"
          height="112"
          rx="8"
          className={
            stage === 'mirroring'
              ? 'fill-signal/20'
              : stage === 'waiting-for-phone'
                ? 'prompt-pulse fill-signal/30'
                : 'fill-paper'
          }
        />
        {stage === 'waiting-for-phone' && (
          <rect x="79" y="102" width="34" height="14" rx="7" className="fill-signal" />
        )}
        {stage === 'mirroring' && (
          <>
            <rect x="78" y="66" width="36" height="6" rx="3" className="fill-signal/60" />
            <rect x="78" y="80" width="26" height="6" rx="3" className="fill-signal/40" />
            <rect x="78" y="126" width="36" height="24" rx="5" className="fill-signal/40" />
          </>
        )}
      </g>

      {/* Computer */}
      <g>
        <rect
          x="558"
          y="44"
          width="160"
          height="108"
          rx="12"
          strokeWidth="3"
          className="fill-panel stroke-ink"
        />
        <rect x="567" y="53" width="142" height="90" rx="6" className="fill-paper" />
        {stage === 'mirroring' && (
          <g>
            <rect x="619" y="58" width="38" height="80" rx="7" className="fill-signal/20 stroke-signal" strokeWidth="2" />
            <rect x="626" y="68" width="24" height="5" rx="2.5" className="fill-signal/60" />
            <rect x="626" y="78" width="17" height="5" rx="2.5" className="fill-signal/40" />
            <rect x="626" y="112" width="24" height="16" rx="4" className="fill-signal/40" />
          </g>
        )}
        <rect x="620" y="152" width="36" height="14" className="fill-ink/80" />
        <rect x="598" y="166" width="80" height="7" rx="3.5" className="fill-ink" />
      </g>
    </svg>
  );
}
