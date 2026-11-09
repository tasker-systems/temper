<script lang="ts">
	import { teamHref } from '$lib/team-view';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();
	let team = $derived(data.team);

	const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
	const day = (iso: string) => iso.slice(0, 10);
</script>

<svelte:head>
	<title>{team.name} — temper</title>
</svelte:head>

<!--
	Every list below renders exactly what `GET /api/teams/{id}` answered to this reader, and an empty
	list renders nothing: the API withholds `children` and `public_within_in` from anyone who does not
	manage the team, and `public_within` from anyone who is not on it, so absence here is the API's
	answer and never a guess this page makes. See `toTeamView` for what the payload carries.
-->
<div class="team mx-auto max-w-3xl px-6 py-8">
	<div class="eyebrow">TEAM · +{team.slug}</div>
	<h1 class="title">{team.name}</h1>
	{#if team.description}
		<p class="mt-2 text-sm text-tp-text-muted">{team.description}</p>
	{/if}

	{#if team.parents.length > 0}
		<section>
			<h2 class="label">Parents</h2>
			<ul>
				{#each team.parents as p (p.id)}
					<li><a href={teamHref(p.id)}>{p.name}</a> <span class="ref">+{p.slug}</span></li>
				{/each}
			</ul>
		</section>
	{/if}

	{#if team.children.length > 0}
		<section>
			<h2 class="label">Child teams</h2>
			<ul>
				{#each team.children as c (c.id)}
					<li>{c.name} <span class="ref">+{c.slug}</span></li>
				{/each}
			</ul>
		</section>
	{/if}

	{#if team.publicWithin.length > 0}
		<section>
			<h2 class="label">Opened within</h2>
			<p class="note">
				This team's own contexts are readable by the people under each team below while its
				opening confers. They can read and reference that material; acting on it still needs
				membership. The audience grows with each team's subtree.
			</p>
			<table>
				<thead>
					<tr><th>Team</th><th>Confers</th><th>Opened</th><th>Audience now</th></tr>
				</thead>
				<tbody>
					{#each team.publicWithin as r (r.anchor.id)}
						<tr>
							<td>{r.anchor.name} <span class="ref">+{r.anchor.slug}</span></td>
							<td>{r.effective ? 'yes' : 'no'}</td>
							<td>{day(r.openedAt)}</td>
							<td>
								{plural(r.audience.principals, 'person', 'people')} on
								{plural(r.audience.teams, 'team', 'teams')}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</section>
	{/if}

	{#if team.publicWithinIn.length > 0}
		<section>
			<h2 class="label">Opened to this team's subtree</h2>
			<table>
				<thead>
					<tr><th>Team</th><th>Confers</th></tr>
				</thead>
				<tbody>
					{#each team.publicWithinIn as r (r.team.id)}
						<tr>
							<td>{r.team.name} <span class="ref">+{r.team.slug}</span></td>
							<td>{r.effective ? 'yes' : 'no'}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</section>
	{/if}

	<section>
		<h2 class="label">Members · {team.members.length}</h2>
		<ul>
			{#each team.members as m (m.profileId)}
				<li>@{m.handle} <span class="ref">{m.role}</span></li>
			{/each}
		</ul>
	</section>
</div>

<style>
	.eyebrow,
	.label,
	.ref,
	th {
		font-family: var(--font-mono);
		letter-spacing: 0.08em;
	}
	.eyebrow {
		font-size: 10px;
		color: var(--tp-text-subtle);
	}
	.title {
		font-size: 1.5rem;
		color: var(--tp-text);
	}
	section {
		margin-top: 1.75rem;
	}
	.label {
		font-size: 10px;
		text-transform: uppercase;
		color: var(--tp-text-muted);
		border-bottom: 1px solid var(--tp-rule-strong);
		padding-bottom: 4px;
		margin-bottom: 8px;
	}
	.note {
		font-size: 0.8rem;
		color: var(--tp-text-subtle);
		margin-bottom: 8px;
	}
	.ref {
		font-size: 10px;
		color: var(--tp-text-subtle);
	}
	ul {
		display: grid;
		gap: 4px;
		font-size: 0.875rem;
		color: var(--tp-text);
	}
	a {
		color: var(--tp-accent);
	}
	table {
		width: 100%;
		font-size: 0.875rem;
		color: var(--tp-text);
		border-collapse: collapse;
	}
	th {
		font-size: 10px;
		text-align: left;
		font-weight: normal;
		color: var(--tp-text-subtle);
		padding: 4px 8px 4px 0;
	}
	td {
		padding: 4px 8px 4px 0;
		border-top: 1px solid var(--tp-rule);
	}
</style>
