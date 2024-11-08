export function compareArraysSimple(arr1: any[], arr2: any[]): boolean {
    if (arr1.length !== arr2.length) return false;
    
    const stringified1 = arr1.map(item => 
        JSON.stringify(Object.entries(item).sort())
    ).sort();
    
    const stringified2 = arr2.map(item => 
        JSON.stringify(Object.entries(item).sort())
    ).sort();
    
    return JSON.stringify(stringified1) === JSON.stringify(stringified2);
}