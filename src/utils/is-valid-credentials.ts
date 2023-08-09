import { TGUserCredentials } from '../types';
import { userCredentialsStruct } from './assert';

export function isValidCredentials(maybeSession: unknown): maybeSession is TGUserCredentials
{
    return userCredentialsStruct(maybeSession).ok;
}
