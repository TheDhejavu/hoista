/**
 * Webhooks: signed event deliveries to an address the person runs. The engine
 * keeps them (`/webhooks`); a new webhook's signing secret is shown once.
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { Webhook, WebhookDelivery } from '@engine-api';
import { apiPath, request } from './request';

/** A webhook just made, or a secret just replaced: the secret is never shown again. */
export interface NewWebhook {
  webhook: Webhook;
  secret: string;
}

const webhookKeys = {
  all: ['webhooks'] as const,
  deliveries: (id: string) => ['webhooks', id, 'deliveries'] as const,
};

export function useWebhooks() {
  return useQuery({
    queryKey: webhookKeys.all,
    queryFn: () => request<Webhook[]>('GET', '/webhooks'),
  });
}

export function useWebhookDeliveries(id: string | null) {
  return useQuery({
    queryKey: webhookKeys.deliveries(id ?? ''),
    enabled: id !== null,
    queryFn: () => request<WebhookDelivery[]>('GET', apiPath('webhooks', id ?? '', 'deliveries')),
  });
}

export function useAddWebhook() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Webhook added' },
    mutationFn: (input: { url: string; events: string[]; project_id: string | null }) =>
      request<NewWebhook>('POST', '/webhooks', input),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: webhookKeys.all }),
  });
}

export function useRotateWebhook() {
  return useMutation({
    meta: { toast: 'New signing secret made' },
    mutationFn: (id: string) => request<NewWebhook>('POST', apiPath('webhooks', id, 'rotate')),
  });
}

export function useRemoveWebhook() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Webhook removed' },
    mutationFn: (id: string) => request<unknown>('DELETE', apiPath('webhooks', id)),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: webhookKeys.all }),
  });
}
