import postgres from 'postgres';
import { DATABASE_URL } from '$env/static/private';

export const sql = postgres(DATABASE_URL, {
	max: 10,
	idle_timeout: 20,
	connect_timeout: 10
});
