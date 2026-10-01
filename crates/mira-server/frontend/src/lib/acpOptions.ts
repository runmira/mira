/** Project an external agent's ACP config options onto Mira's own
 *  `OptionDescriptor` shape.
 *
 *  The point is that the model picker then needs no knowledge of ACP: ACP
 *  has no set-model method, so a model *is* a config option with
 *  `category: "model"`. Converting here means one renderer serves both Mira's
 *  own capability-derived descriptors and an agent's.
 *
 *  Kept pure and in its own module so it is testable without a socket.
 */
import type { OptionDescriptor } from '../api';
import type { AcpConfigOption } from '../types';

/** Human label for a config category. Unknown categories keep their slug so
 *  nothing is silently hidden — an unrecognised option is still actionable. */
function labelFor(option: AcpConfigOption): string {
  switch (option.category) {
    case 'model': return 'Model';
    case 'thought_level': return 'Thinking';
    case 'reasoning_effort': return 'Reasoning';
    case 'mode': return 'Mode';
    case 'permission_mode': return 'Permissions';
    default: return option.name || option.id;
  }
}

/**
 * Convert, or `null` when the agent offered nothing actionable.
 *
 * Returns `null` rather than an empty array so a caller can distinguish
 * "this agent has no options" from "this agent has options and none apply"
 * — the first means the picker should show Mira's own, the second means the
 * agent's list is authoritative.
 */
export function acpOptionsToDescriptors(
  options: AcpConfigOption[] | null | undefined,
): OptionDescriptor[] | null {
  if (!options || options.length === 0) return null;

  const out: OptionDescriptor[] = [];
  for (const o of options) {
    if (o.values && o.values.length > 0) {
      out.push({
        type: 'select',
        id: o.id,
        label: labelFor(o),
        options: o.values.map((v) => ({
          value: v.value,
          // `name` is the agent's own label; fall back to the raw value
          // rather than showing an empty row.
          label: v.name || v.value,
        })),
      });
    } else if (o.category === 'boolean' || o.values === undefined) {
      // A toggle has no `values`; ACP models it as a select over
      // "enabled"/"disabled" or as a boolean kind. We only claim it when the
      // agent actually gave us two choices, so we never invent a control.
      continue;
    }
  }
  return out.length > 0 ? out : null;
}

/** The value an option should start at, or `null` if it has no current. */
export function acpInitialValue(o: AcpConfigOption): string | null {
  return o.current ?? null;
}
