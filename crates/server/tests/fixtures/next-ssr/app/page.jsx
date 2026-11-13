import { headers } from 'next/headers';
export const dynamic = 'force-dynamic';
export default async function Page() {
  const request = await headers();
  return <main><h1>Hoista SSR fixture</h1><p>hoista-server-only-fixture</p><span>{request.get('host')}</span></main>;
}
