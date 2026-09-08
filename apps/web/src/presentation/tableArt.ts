/** Initial, bounded selection of published imagery. Provenance: public/table-art/sources.json. */
export interface CardArt { file: string; crop?: [number, number, number, number] }
export const cardArt: Record<string, CardArt> = {
  'starter:001': { file: 'alohomora.jpg', crop: [18, 18, 135, 140] },
  'starter:002': { file: 'starting-cards.jpg', crop: [27, 27, 84, 72] },
  'starter:004': { file: 'starting-cards.jpg', crop: [132, 48, 82, 74] },
  'starter:011': { file: 'starting-cards.jpg', crop: [257, 30, 87, 65] },
  'starter:012': { file: 'starting-cards.jpg', crop: [356, 55, 81, 74] },
  'hogwarts-card:005': { file: 'incendio.png' },
  'hogwarts-card:009': { file: 'hogwarts-cards.png', crop: [177, 49, 127, 74] },
  'hogwarts-card:010': { file: 'hogwarts-cards.png', crop: [323, 49, 124, 77] },
  'hogwarts-card:013': { file: 'hogwarts-cards.png', crop: [25, 52, 123, 74] },
  'villain:001': { file: 'crabbe-goyle.png' },
  'villain:002': { file: 'draco.png' },
  'villain:003': { file: 'quirrell.png' },
  'location:001': { file: 'diagon-alley.png' },
  'prototype-spell': { file: 'alohomora.jpg' },
  'prototype-owl': { file: 'starting-cards.jpg', crop: [132, 48, 82, 74] },
  'prototype-draco': { file: 'draco.png' },
  'prototype-location': { file: 'diagon-alley.png' },
}
