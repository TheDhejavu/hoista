/**
 * A route parameter the screen can't work without, such as `:appId`.
 * Throws if the route doesn't define it, which is a bug in the route table.
 */
import { useParams } from 'react-router-dom';

export function useRequiredParam(name: string): string {
  const value = useParams()[name];
  if (!value) throw new Error(`This screen needs a :${name} in its route.`);
  return value;
}
