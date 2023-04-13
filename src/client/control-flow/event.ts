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

    listenerCount(): number 
    {
        return this._listeners.length;
    }

    on(cb: TGEventCb<T, U, V>): TGEvent<T, U, V> 
    {
        if (this._listeners.indexOf(cb) !== -1) 
        {
            return this;
        }
        this._listeners.push(cb);
        return this;
    }

    off(cb: TGEventCb<T, U, V>): TGEvent<T, U, V> 
    {
        const idx = this._listeners.indexOf(cb);
        if (idx !== -1) 
        {
            this._listeners.splice(idx, 1);
        }
        return this;
    }

    reset(): TGEvent<T, U, V> 
    {
        this._listeners = [];
        return this;
    }

    trigger(a: T, b?: U, c?: V): TGEvent<T, U, V> 
    {
        this._listeners.forEach(cb => cb(a, b, c));
        return this;
    }
}
