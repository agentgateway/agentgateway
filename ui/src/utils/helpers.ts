/**
 * Capitalizes the first letter of each word in a string (e.g. "hello world" -> "Hello World").
 */
export const capitalizeFirstLetters = (s: string) => {
  return s
    .split(' ')
    .map(s => (s.at(0)?.toLocaleUpperCase() ?? '') + s.slice(1))
    .join(' ');
};