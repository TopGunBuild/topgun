export const CHANGELOG_SOUL = 'changelog';
export const PEER_SYNC_SOUL = 'peersync';

export const DEFAULT_FEDERATION_OPTIONS = {
    backSync         : 1000 * 60 * 60 * 24, // 24 hours
    batchInterval    : 500,
    maintainChangelog: true,
    maxStaleness     : 1000 * 60 * 60 * 24,
    putToPeers       : false
};
