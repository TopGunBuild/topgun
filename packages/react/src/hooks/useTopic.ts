import { useEffect, useRef } from 'react';
import { TopicCallback } from '@topgunbuild/client';
import { useClient } from './useClient';

export function useTopic(topicName: string, callback?: TopicCallback) {
    const client = useClient();
    const topic = client.topic(topicName);
    const isMounted = useRef(true);

    // Keep callback ref stable to avoid re-subscribing if callback function identity changes
    const callbackRef = useRef(callback);
    useEffect(() => {
        callbackRef.current = callback;
    }, [callback]);

    useEffect(() => {
        isMounted.current = true;

        if (!callback) return;

        const unsubscribe = topic.subscribe((data, context) => {
            if (isMounted.current && callbackRef.current) {
                callbackRef.current(data, context);
            }
        });

        return () => {
            isMounted.current = false;
            unsubscribe();
        };
    }, [topic, callback]); // Re-subscribe if topic handle changes (rare) or if callback presence toggles

    return topic;
}
