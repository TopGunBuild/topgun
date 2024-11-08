export function wait(duration = 0): Promise<void>
{
    return new Promise((resolve) =>
    {
        setTimeout(() => resolve(), duration);
    });
}
