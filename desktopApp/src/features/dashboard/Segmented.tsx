interface SegmentedProps<V extends string> {
  label: string;
  value: V;
  options: readonly { value: V; label: string }[];
  onChange: (value: V) => void;
}

/**
 * A row of mutually exclusive choices, all visible at once.
 *
 * Used instead of a dropdown because the choices are few and comparing them is
 * the point: "1080p or 2K?" is easier with both on screen than behind a click.
 */
export function Segmented<V extends string>({ label, value, options, onChange }: SegmentedProps<V>) {
  return (
    <div role="radiogroup" aria-label={label} className="flex flex-wrap gap-1.5">
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={selected}
            onClick={() => onChange(option.value)}
            className={`rounded-lg border px-3 py-1.5 text-sm font-medium transition-colors ${
              selected
                ? 'border-signal bg-signal/10 text-signal'
                : 'border-line text-ink hover:border-ink/40'
            }`}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
