declare global {
	namespace App {
		interface Locals {
			user: { profileId: string; email: string; displayName: string } | null;
			accessToken: string | null;
		}
	}
}

export {};
