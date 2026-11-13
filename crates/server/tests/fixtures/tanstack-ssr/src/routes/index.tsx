import { createFileRoute } from '@tanstack/react-router';
import { createServerFn } from '@tanstack/react-start';
const readMessage = createServerFn({ method: 'GET' }).handler(() => 'hoista-server-only-fixture');
export const Route = createFileRoute('/')({ loader: () => readMessage(), component: Page });
function Page() { return <main><h1>Hoista SSR fixture</h1><p>{Route.useLoaderData()}</p><a href="/fixture.txt">Static file</a></main>; }
