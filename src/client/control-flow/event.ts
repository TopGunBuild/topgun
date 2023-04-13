type TGEventCb<T = any, U = any, V = any> = (a: T, b?: U, c?: V) => void;

/**
 * Generic event/listener system
 */
export class TGEvent<T = any, U = any, V = any> 
{
    readonly name: string;
    private _listeners: Array<TGEventCb<T, U, V>>;

    /**
     * Constructor
     */
    constructor(name = 'Event') 
    {
        this.name = name;
        this._listeners = [];
        this.listenerCount = this.listenerCount.bind(this);
        this.on = this.on.bind(this);
        this.off = this.off.bind(this);
        this.trigger = this.trigger.bind(this);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * @returns number of currently subscribed listeners
     */
    listenerCount(): number 
    {
        return this._listeners.length;
    }

    /**
     * Register a listener on this event
     *
     * @param cb the callback to subscribe
     */
    on(cb: TGEventCb<T, U, V>): TGEvent<T, U, V> 
    {
        if (this._listeners.indexOf(cb) !== -1) 
        {
            return this;
        }
        this._listeners.push(cb);
        return this;
    }

    /**
     * Unregister a listener on this event
     * @param cb the callback to unsubscribe
     */
    off(cb: TGEventCb<T, U, V>): TGEvent<T, U, V> 
    {
        const idx = this._listeners.indexOf(cb);
        if (idx !== -1) 
        {
            this._listeners.splice(idx, 1);
        }
        return this;
    }

    /**
     * Unregister all listeners on this event
     */
    reset(): TGEvent<T, U, V> 
    {
        this._listeners = [];
        return this;
    }

    /**
     * Trigger this event
     */
    trigger(a: T, b?: U, c?: V): TGEvent<T, U, V> 
    {
        this._listeners.forEach(cb => cb(a, b, c));
        return this;
    }
}
