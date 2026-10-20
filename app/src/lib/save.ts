import { Directory, File, Paths } from 'expo-file-system';
import { Image } from 'expo-image';
import { Asset, requestPermissionsAsync } from 'expo-media-library';
import * as Sharing from 'expo-sharing';

import { authHeaders, type Connection, type JobImage } from './api';

const folder = () => new Directory(Paths.cache, 'images');

/** The full-size PNG in the app's cache, downloaded once with the password. */
async function download(connection: Connection, jobId: string, image: JobImage): Promise<File> {
  const dir = folder();
  if (!dir.exists) dir.create({ intermediates: true });
  const target = new File(dir, `${jobId}-${image.filename.replace(/[^\w.-]/g, '_')}`);
  if (target.exists) return target;
  return File.downloadFileAsync(connection.address + image.url, target, { headers: authHeaders(connection), idempotent: true });
}

export async function saveToPhotos(connection: Connection, jobId: string, image: JobImage): Promise<void> {
  // Add-only access: the app can save images but never see the photo library.
  const permission = await requestPermissionsAsync(true);
  if (!permission.granted) {
    throw new Error('Saving needs permission to add photos. Allow it for this app in the Settings app, under Privacy & Security › Photos.');
  }
  const file = await download(connection, jobId, image);
  await Asset.create(file.uri);
}

export async function shareImage(connection: Connection, jobId: string, image: JobImage): Promise<void> {
  const file = await download(connection, jobId, image);
  await Sharing.shareAsync(file.uri, { mimeType: 'image/png', UTI: 'public.png' });
}

/** Remove every image this app keeps: downloads and expo-image's caches. */
export async function clearImages(): Promise<void> {
  try {
    const dir = folder();
    if (dir.exists) dir.delete();
  } catch {
    // Already gone.
  }
  await Promise.all([Image.clearDiskCache(), Image.clearMemoryCache()]);
}
