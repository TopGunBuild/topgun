import React, { useState, useEffect } from 'react';
import { Bell, BellOff, BellRing, Loader2 } from 'lucide-react';
import {
  isPushSupported,
  isSubscribed,
  subscribeToPush,
  unsubscribeFromPush,
  getPermissionStatus,
} from '../lib/pushNotifications';

interface PushNotificationToggleProps {
  userId: string;
  darkMode: boolean;
  compact?: boolean;
}

type SubscriptionState = 'loading' | 'unsupported' | 'denied' | 'subscribed' | 'unsubscribed';

export function PushNotificationToggle({ userId, darkMode, compact = false }: PushNotificationToggleProps) {
  const [state, setState] = useState<SubscriptionState>('loading');
  const [isProcessing, setIsProcessing] = useState(false);

  useEffect(() => {
    checkSubscriptionState();
  }, []);

  async function checkSubscriptionState() {
    if (!isPushSupported()) {
      setState('unsupported');
      return;
    }

    const permission = getPermissionStatus();
    if (permission === 'denied') {
      setState('denied');
      return;
    }

    const subscribed = await isSubscribed();
    setState(subscribed ? 'subscribed' : 'unsubscribed');
  }

  async function handleToggle() {
    if (isProcessing) return;

    setIsProcessing(true);
    try {
      if (state === 'subscribed') {
        await unsubscribeFromPush(userId);
        setState('unsubscribed');
      } else {
        const result = await subscribeToPush(userId);
        if (result) {
          setState('subscribed');
        } else {
          // Permission was denied
          const permission = getPermissionStatus();
          setState(permission === 'denied' ? 'denied' : 'unsubscribed');
        }
      }
    } catch (error) {
      console.error('Push notification toggle error:', error);
    } finally {
      setIsProcessing(false);
    }
  }

  const theme = {
    text: darkMode ? 'text-gray-100' : 'text-gray-800',
    textSecondary: darkMode ? 'text-gray-400' : 'text-gray-600',
    hover: darkMode ? 'hover:bg-gray-700' : 'hover:bg-gray-100',
    bg: darkMode ? 'bg-gray-700' : 'bg-gray-100',
    bgActive: darkMode ? 'bg-blue-600' : 'bg-blue-500',
  };

  // Compact mode - just an icon button
  if (compact) {
    if (state === 'loading') {
      return (
        <div className={`p-2 ${theme.bg} rounded-lg`}>
          <Loader2 size={20} className={`${theme.textSecondary} animate-spin`} />
        </div>
      );
    }

    if (state === 'unsupported') {
      return null;
    }

    if (state === 'denied') {
      return (
        <div className={`p-2 ${theme.bg} rounded-lg cursor-not-allowed`} title="Уведомления заблокированы в браузере">
          <BellOff size={20} className="text-red-400" />
        </div>
      );
    }

    return (
      <button
        onClick={handleToggle}
        disabled={isProcessing}
        className={`p-2 ${state === 'subscribed' ? theme.bgActive : theme.bg} ${theme.hover} rounded-lg transition-colors`}
        title={state === 'subscribed' ? 'Отключить уведомления' : 'Включить уведомления'}
      >
        {isProcessing ? (
          <Loader2 size={20} className="text-white animate-spin" />
        ) : state === 'subscribed' ? (
          <BellRing size={20} className="text-white" />
        ) : (
          <Bell size={20} className={theme.textSecondary} />
        )}
      </button>
    );
  }

  // Full mode - button with label
  if (state === 'loading') {
    return (
      <div className={`flex items-center gap-3 p-3 ${theme.bg} rounded-lg`}>
        <Loader2 size={20} className={`${theme.textSecondary} animate-spin`} />
        <span className={`text-sm ${theme.textSecondary}`}>Загрузка...</span>
      </div>
    );
  }

  if (state === 'unsupported') {
    return (
      <div className={`flex items-center gap-3 p-3 ${theme.bg} rounded-lg`}>
        <BellOff size={20} className={theme.textSecondary} />
        <span className={`text-sm ${theme.textSecondary}`}>
          Уведомления не поддерживаются
        </span>
      </div>
    );
  }

  if (state === 'denied') {
    return (
      <div className={`flex items-center gap-3 p-3 ${theme.bg} rounded-lg`}>
        <BellOff size={20} className="text-red-400" />
        <div className="flex flex-col">
          <span className={`text-sm ${theme.text}`}>Уведомления заблокированы</span>
          <span className={`text-xs ${theme.textSecondary}`}>
            Разрешите в настройках браузера
          </span>
        </div>
      </div>
    );
  }

  return (
    <button
      onClick={handleToggle}
      disabled={isProcessing}
      className={`flex items-center gap-3 p-3 w-full ${
        state === 'subscribed' ? theme.bgActive : theme.bg
      } ${theme.hover} rounded-lg transition-colors`}
    >
      {isProcessing ? (
        <Loader2 size={20} className={state === 'subscribed' ? 'text-white animate-spin' : `${theme.textSecondary} animate-spin`} />
      ) : state === 'subscribed' ? (
        <BellRing size={20} className="text-white" />
      ) : (
        <Bell size={20} className={theme.textSecondary} />
      )}
      <div className="flex flex-col items-start">
        <span className={`text-sm font-medium ${state === 'subscribed' ? 'text-white' : theme.text}`}>
          {state === 'subscribed' ? 'Уведомления включены' : 'Включить уведомления'}
        </span>
        <span className={`text-xs ${state === 'subscribed' ? 'text-blue-100' : theme.textSecondary}`}>
          {state === 'subscribed' ? 'Нажмите, чтобы отключить' : 'Напоминания о заметках'}
        </span>
      </div>
    </button>
  );
}
