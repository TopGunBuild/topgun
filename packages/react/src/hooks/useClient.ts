// Re-export useClient from hooks folder for API consistency (useClient lives
// in TopGunProvider.tsx but is surfaced here so consumers can import from
// '@topgunbuild/react' without knowing the provider file layout).
export { useClient } from '../TopGunProvider';
