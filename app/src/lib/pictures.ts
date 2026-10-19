import { File, UploadType } from 'expo-file-system';
import * as ImagePicker from 'expo-image-picker';

import { ApiError, authHeaders, unreachableMessage, type Connection } from './api';

export type Picked = { uri: string; width: number; height: number };

/**
 * One picture from the photo library. The system picker runs outside the app, so this needs no
 * access to the library. HEIC photos come back as JPEG, which ComfyUI reads.
 */
export async function pickPicture(): Promise<Picked | null> {
  const result = await ImagePicker.launchImageLibraryAsync({
    mediaTypes: ['images'],
    quality: 0.9,
    preferredAssetRepresentationMode: ImagePicker.UIImagePickerPreferredAssetRepresentationMode.Compatible,
    shouldDownloadFromNetwork: true,
  });
  if (result.canceled || result.assets.length === 0) return null;
  const { uri, width, height } = result.assets[0];
  return { uri, width, height };
}

/** Send a picked picture to the Mac. Resolves to the name an edit asks for it by. */
export async function uploadPicture(connection: Connection, uri: string): Promise<string> {
  let result;
  try {
    result = await new File(uri).upload(`${connection.address}/api/uploads`, {
      uploadType: UploadType.BINARY_CONTENT,
      headers: { ...authHeaders(connection), 'Content-Type': 'application/octet-stream' },
    });
  } catch {
    throw new ApiError(unreachableMessage(connection.address), 0);
  }
  let body: { upload?: string; error?: string } | null = null;
  try {
    body = JSON.parse(result.body);
  } catch {
    // The status says enough.
  }
  if (result.status !== 201 || !body?.upload) {
    throw new ApiError(body?.error ?? `The server answered ${result.status}.`, result.status);
  }
  return body.upload;
}
