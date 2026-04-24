import { waitForPg } from './setup';

export default async function globalSetup(): Promise<void> {
  await waitForPg();
}
