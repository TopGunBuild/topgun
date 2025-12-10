import { useClient } from '../TopGunProvider';

// Re-export useClient from hooks for consistency if preferred,
// or just keep it in provider file. The prompt listed `useClient()` separately
// in requirements, but usually it lives with the Context.
// I'll create a simple re-export here if I decide to separate it,
// but for now I implemented it in TopGunProvider.tsx.
// I will create this file to export it from hooks folder as well.

export { useClient } from '../TopGunProvider';

