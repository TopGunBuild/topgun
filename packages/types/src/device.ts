import { Keyset, KeysetWithSecrets } from './keyset';

export type DeviceInfo = {
    userId: string;
    deviceId: string;
    deviceName: string;
    deviceInfo?: any;
    created?: bigint;
}

export type DeviceWithSecrets = {
    keys: KeysetWithSecrets
}&DeviceInfo

export type Device = {
    keys: Keyset
}&DeviceInfo

export type FirstUseDeviceWithSecrets = Omit<DeviceWithSecrets, 'userId'>
export type FirstUseDevice = Omit<Device, 'userId'>
