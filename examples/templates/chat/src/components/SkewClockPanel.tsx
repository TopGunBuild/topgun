/**
 * Demo-only incoming-message delay simulation. Does NOT skew the client's
 * outgoing HLC — no such public API exists on TopGunClient. When the +5s
 * toggle is active, incoming messages are buffered for 5 seconds before being
 * added to the visible message list. This lets a visitor observe a
 * late-arriving message slot into its correct HLC-causal position upon
 * delivery, demonstrating that TopGun's ordering is timestamp-based, not
 * arrival-order-based.
 *
 * When a public clock-offset API is available on TopGunClient, this panel
 * can buffer outgoing timestamps instead of incoming ones.
 */

interface SkewClockPanelUIProps {
  skewEnabled: boolean;
  onToggle: (enabled: boolean) => void;
  bufferedCount: number;
}

/**
 * UI strip for the skew-clock demo. Always shows the mandatory label.
 */
export function SkewClockPanelUI({ skewEnabled, onToggle, bufferedCount }: SkewClockPanelUIProps) {
  return (
    <div className="bg-amber-50 border border-amber-200 rounded-lg px-4 py-3">
      <p className="text-xs text-amber-700 font-medium mb-2">
        Demo-only: simulates incoming message delay — not real HLC skew.
      </p>
      <div className="flex items-center gap-3">
        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={skewEnabled}
            onChange={(e) => onToggle(e.target.checked)}
            className="h-4 w-4 rounded border-amber-300 text-amber-600 cursor-pointer"
          />
          <span className="text-sm text-amber-800">Delay incoming messages by +5s</span>
        </label>
        {skewEnabled && bufferedCount > 0 && (
          <span className="text-xs text-amber-600">
            {bufferedCount} message{bufferedCount !== 1 ? 's' : ''} buffered…
          </span>
        )}
      </div>
    </div>
  );
}
