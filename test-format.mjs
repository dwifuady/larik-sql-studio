import { formatSqlWithIndentation } from './src/utils/sqlFormatter';

const query = `SELECT top(100) * from [dbo].[Application] AS [a3]
join [dbo].[ApplicationStatus] AS [as1] on [a3].[Status] = [as1].[Id]
join [dbo].[BCApplicationState] AS [bcas] on [a3].[BCState] = [bcas].[Id]
join [dbo].[BCApplicationStep] AS [stp] on [stp].[ID] = [a3].[lastStep]
WHERE [a3].[Status] = 3;`;

console.log('Input:');
console.log(query);
console.log('\n=== Formatted Output ===\n');
const formatted = formatSqlWithIndentation(query);
console.log(formatted);
