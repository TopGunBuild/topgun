export interface TLSConfig {
    /**
     * Enable TLS for client-facing server (HTTPS + WSS)
     * @default false
     */
    enabled: boolean;

    /**
     * Path to certificate file (PEM format)
     * Supports chain certificates
     */
    certPath: string;

    /**
     * Path to private key file (PEM format)
     */
    keyPath: string;

    /**
     * Path to CA certificate for verifying client certificates
     * Required for mTLS
     * @optional
     */
    caCertPath?: string;

    /**
     * Minimum TLS version
     * @default 'TLSv1.2'
     */
    minVersion?: 'TLSv1.2' | 'TLSv1.3';

    /**
     * List of allowed cipher suites
     * @optional - use Node.js defaults if not specified
     */
    ciphers?: string;

    /**
     * Passphrase for encrypted private key
     * @optional
     */
    passphrase?: string;
}

export interface ClusterTLSConfig extends TLSConfig {
    /**
     * Require client certificate (mTLS)
     * @default false
     */
    requireClientCert?: boolean;

    /**
     * Verify peer certificates
     * Can be disabled in development for self-signed certs
     * @default true
     */
    rejectUnauthorized?: boolean;
}
