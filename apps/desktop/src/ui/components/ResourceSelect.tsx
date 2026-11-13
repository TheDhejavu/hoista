/**
 * The Resource dropdown every project tab shares: the project's resources grouped
 * by provider, in the order the app lists them, with the chosen one's provider logo
 * on the pill. `all` adds an "All" choice for tabs that can show everything.
 */
import type { Resource } from '@shared/api';
import { serviceName } from '@shared/providers';
import { resourceKindLabel } from '@ui/screens/project/resourceKinds';
import { PillSelect, type Option } from './Select';
import { ServiceIcon } from './ServiceIcon';

export function ResourceSelect<T extends string | null>({
  resources,
  value,
  onChange,
  all = false,
  disabled,
  note,
}: {
  resources: readonly Resource[];
  value: T;
  onChange: (value: T) => void;
  /** Offer "All" (the value null) first. */
  all?: boolean;
  /** Resources that are listed but can't be picked here. */
  disabled?: (resource: Resource) => boolean;
  /** A few words after a resource that can't be picked, such as why. */
  note?: (resource: Resource) => string | null;
}) {
  const order = (resource: Resource) =>
    resources.findIndex((item) => item.provider === resource.provider);
  const sorted = [...resources].sort(
    (a, b) =>
      order(a) - order(b) || Number(disabled?.(a) ?? false) - Number(disabled?.(b) ?? false),
  );
  const options: Option<T>[] = [
    ...(all ? [{ value: null as T, label: 'All' }] : []),
    ...sorted.map((resource) => {
      const off = disabled?.(resource) ?? false;
      const why = off ? note?.(resource) : null;
      return {
        value: resource.id as T,
        label: `${resource.name} · ${resourceKindLabel(resource.kind)}${why ? ` · ${why}` : ''}`,
        group: serviceName(resource.provider),
        disabled: off,
      };
    }),
  ];
  const chosen = resources.find((resource) => resource.id === value);
  return (
    <PillSelect
      label="Resource"
      value={value}
      options={options}
      onChange={onChange}
      icon={chosen ? <ServiceIcon id={chosen.provider} size={14} /> : undefined}
      display={(option) => (option.value === null ? 'All' : (chosen?.name ?? option.label))}
    />
  );
}
