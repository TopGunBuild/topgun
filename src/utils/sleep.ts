export async function sleep(duration = 1000): Promise<void>
{
    return new Promise(ok => setTimeout(ok, duration));
}
