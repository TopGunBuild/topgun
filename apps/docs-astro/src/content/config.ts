import { defineCollection, z } from 'astro:content';

const docs = defineCollection({
    schema: z.object({
        title: z.string(),
        description: z.string().optional(),
        order: z.number().optional(),
    }),
});

const blog = defineCollection({
    schema: z.object({
        title: z.string(),
        excerpt: z.string(),
        date: z.string(),
        readTime: z.string(),
        author: z.string(),
        role: z.string(),
        category: z.string(),
        image: z.string(),
    }),
});

export const collections = { docs, blog };
