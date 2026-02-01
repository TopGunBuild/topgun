import { z } from 'zod';

const EnvSchema = z
    .object({
        // Required
        NODE_ENV: z
            .enum(['development', 'test', 'production'])
            .default('development'),

        // Server ports
        TOPGUN_PORT: z.coerce
            .number()
            .int()
            .min(1)
            .max(65535)
            .default(8080),
        TOPGUN_CLUSTER_PORT: z.coerce
            .number()
            .int()
            .min(1)
            .max(65535)
            .default(9080),
        TOPGUN_METRICS_PORT: z.coerce
            .number()
            .int()
            .min(1)
            .max(65535)
            .optional(),

        // Node identity
        NODE_ID: z.string().optional(),

        // Clustering
        TOPGUN_PEERS: z.string().optional(),
        TOPGUN_DISCOVERY_SERVICE: z.string().optional(),
        TOPGUN_DISCOVERY_INTERVAL: z.coerce
            .number()
            .int()
            .positive()
            .default(10000),

        // Database
        DATABASE_URL: z.string().url().optional(),

        // Security
        JWT_SECRET: z.string().min(32).optional(),

        // TLS - Client facing
        TOPGUN_TLS_ENABLED: z
            .enum(['true', 'false'])
            .default('false')
            .transform((v) => v === 'true'),
        TOPGUN_TLS_CERT_PATH: z.string().optional(),
        TOPGUN_TLS_KEY_PATH: z.string().optional(),
        TOPGUN_TLS_CA_PATH: z.string().optional(),
        TOPGUN_TLS_MIN_VERSION: z
            .enum(['TLSv1.2', 'TLSv1.3'])
            .default('TLSv1.2'),
        TOPGUN_TLS_PASSPHRASE: z.string().optional(),

        // TLS - Cluster
        TOPGUN_CLUSTER_TLS_ENABLED: z
            .enum(['true', 'false'])
            .default('false')
            .transform((v) => v === 'true'),
        TOPGUN_CLUSTER_TLS_CERT_PATH: z.string().optional(),
        TOPGUN_CLUSTER_TLS_KEY_PATH: z.string().optional(),
        TOPGUN_CLUSTER_TLS_CA_PATH: z.string().optional(),
        TOPGUN_CLUSTER_MTLS: z
            .enum(['true', 'false'])
            .default('false')
            .transform((v) => v === 'true'),
        TOPGUN_CLUSTER_TLS_REJECT_UNAUTHORIZED: z
            .enum(['true', 'false'])
            .default('true')
            .transform((v) => v === 'true'),

        // Debug
        TOPGUN_DEBUG: z
            .enum(['true', 'false'])
            .default('false')
            .transform((v) => v === 'true'),
    })
    .superRefine((data, ctx) => {
        // Production requirements
        if (data.NODE_ENV === 'production') {
            if (!data.JWT_SECRET) {
                ctx.addIssue({
                    code: z.ZodIssueCode.custom,
                    message: 'JWT_SECRET is required in production',
                    path: ['JWT_SECRET'],
                });
            }
        }

        // TLS cert/key pairs
        if (data.TOPGUN_TLS_ENABLED) {
            if (!data.TOPGUN_TLS_CERT_PATH) {
                ctx.addIssue({
                    code: z.ZodIssueCode.custom,
                    message: 'TOPGUN_TLS_CERT_PATH required when TLS enabled',
                    path: ['TOPGUN_TLS_CERT_PATH'],
                });
            }
            if (!data.TOPGUN_TLS_KEY_PATH) {
                ctx.addIssue({
                    code: z.ZodIssueCode.custom,
                    message: 'TOPGUN_TLS_KEY_PATH required when TLS enabled',
                    path: ['TOPGUN_TLS_KEY_PATH'],
                });
            }
        }

        if (data.TOPGUN_CLUSTER_TLS_ENABLED) {
            if (!data.TOPGUN_CLUSTER_TLS_CERT_PATH && !data.TOPGUN_TLS_CERT_PATH) {
                ctx.addIssue({
                    code: z.ZodIssueCode.custom,
                    message:
                        'Cluster TLS requires cert path (TOPGUN_CLUSTER_TLS_CERT_PATH or TOPGUN_TLS_CERT_PATH)',
                    path: ['TOPGUN_CLUSTER_TLS_CERT_PATH'],
                });
            }
            if (!data.TOPGUN_CLUSTER_TLS_KEY_PATH && !data.TOPGUN_TLS_KEY_PATH) {
                ctx.addIssue({
                    code: z.ZodIssueCode.custom,
                    message:
                        'Cluster TLS requires key path (TOPGUN_CLUSTER_TLS_KEY_PATH or TOPGUN_TLS_KEY_PATH)',
                    path: ['TOPGUN_CLUSTER_TLS_KEY_PATH'],
                });
            }
        }
    });

export type EnvConfig = z.infer<typeof EnvSchema>;

export function validateEnv(): EnvConfig {
    const result = EnvSchema.safeParse(process.env);
    if (!result.success) {
        const errors = result.error.issues
            .map((e) => `  - ${e.path.join('.')}: ${e.message}`)
            .join('\n');
        console.error(`Environment validation failed:\n${errors}`);
        process.exit(1);
    }
    return result.data;
}
