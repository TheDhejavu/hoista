import { providerEntry } from '@shared/providers';
import type { Money } from '@shared/api';

/** What each provider calls the things Hoista finds there. */
export function projectsWord(service: string): string {
  return providerEntry(service)?.projectsWord ?? 'Projects';
}

export function dollars(money: Money | null | undefined): number | null {
  return money ? Math.round(money.cents / 100) : null;
}

/** `DEPLOYMENT_FAILED`, `Deployment.failed` or `HOBBY` → "Deployment failed", "Hobby". */
export function words(text: string): string {
  const spaced = text
    .replace(/[._-]+/g, ' ')
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .trim()
    .toLowerCase();
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

export function channelWord(type: string): string {
  switch (type.toUpperCase()) {
    case 'EMAIL':
      return 'Email';
    case 'INAPP':
      return 'In project';
    case 'WEBHOOK':
      return 'Webhook';
    default:
      return words(type);
  }
}

/** A provider's value: its own names ("Sam's Projects") stay; codes ("OWNER", "pro") read as words. */
export function providerWords(text: string): string {
  return /^[A-Z_]+$|^[a-z_]+$/.test(text) ? words(text) : text;
}
