import axios from "axios";

export async function fetchVaultSummary(): Promise<string> {
  const response = await axios.get<string>("/api/vault");
  return response.data;
}
