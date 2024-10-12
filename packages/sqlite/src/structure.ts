/** The root action's payload is defined by the application. */
export type RootAction = {
    type: 'ROOT'
    payload: any
}

/**
 * An `Action` is analogous to a Redux action: it has a string label (e.g. 'ADD_USER' or
 * 'INCREMENT') and a payload that can contain anything. The application will narrow this down
 * by defining a union of all the possible actions.
 */
export type Action =
    |RootAction
    |{
    /** Label identifying the type of action this link represents */
    type: string

    /** Payload of the action */
    payload: any
}

/** The `LinkBody` adds contextual information to the `Action`. This is the part of the link that is encrypted. */
export type LinkBody<A extends Action, C> = {
    /** User who authored this link */
    userId: string

    /** Unix timestamp on device that created this link */
    timestamp: BigInt

    /** Head(s) of the graph when this link was added */
    prev: string[]
}&A& // plus everything from the action interface
    C // plus everything from the context interface

/** A link consists of a body, as well as a hash calculated from the body. */
export type Link<A extends Action, C> = {
    /** Hash of the body */
    hash: string

    /** The part of the link that is encrypted */
    body: LinkBody<A, C>

    isInvalid?: boolean
}
