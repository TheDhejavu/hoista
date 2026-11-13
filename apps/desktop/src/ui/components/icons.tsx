/**
 * The Hoista logo: the mascot standing with its bar, or its faceless mark
 * when there's too little room for a face. Every other icon comes from
 * `lucide-react`, used directly (`import { Search } from 'lucide-react'`) at
 * 15-16px with the default stroke.
 */
import { Mascot } from './Mascot';

export function LogoMark({ size = 18 }: { size?: number }) {
  return <Mascot mood={size < 22 ? 'mark' : 'logo'} size={size} />;
}
