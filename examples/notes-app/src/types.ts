export interface Attachment {
  id: string;
  name: string;
  type: string;
  size: number;
  url: string;
  storageKey?: string;
}

export interface Note {
  id: string;
  title: string;
  content: string;
  updatedAt: number;
  isFavorite: boolean;
  folderId: string;
  date?: string;
  time?: string;
  recurring?: 'none' | 'daily' | 'weekly' | 'monthly';
  attachments?: Attachment[];
}

export interface Folder {
  id: string;
  name: string;
}

