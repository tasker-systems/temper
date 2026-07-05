// territory.ts

/** A territory with no members — rendered as a de-emphasized ghost (L3).
 *  Structural param so both `Territory` and `PositionedTerritory` satisfy it. */
export function isEmptyTerritory(t: { member_count: number }): boolean {
	return t.member_count === 0;
}
