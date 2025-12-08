using DotnetEmail;
using Google.Apis.Gmail.v1;
using Google.Apis.Gmail.v1.Data;

const int MaxThreads = 20;

Console.WriteLine("Spam Folder Email Counter");

// Parse optional days parameter from command line arguments
int? daysLimit = null;
if (args.Length > 0 && int.TryParse(args[0], out int days))
{
    daysLimit = days;
    Console.WriteLine($"Limiting to emails from the last {days} day(s), i.e., since {DateTimeOffset.Now.AddDays(-days)}");
}

try
{
    var service = await GmailServiceHelper.GetGmailService();

    // Define parameters of request to get spam messages
    UsersResource.MessagesResource.ListRequest request = service.Users.Messages.List("me");
    request.LabelIds = new List<string> { "SPAM" };
    request.IncludeSpamTrash = true;
    request.MaxResults = 500; // Fetch more messages at once
    request.Q = daysLimit.HasValue ? $"after:{DateTimeOffset.UtcNow.AddDays(-daysLimit.Value).ToUnixTimeSeconds()}" : ""; // Query to fetch spam messages after a certain date

    // Dictionary to store date counts
    var dateCountMap = new Dictionary<DateOnly, int>();

    // Calculate cutoff date if days limit is specified
    DateTimeOffset? cutoffDate = null;
    if (daysLimit.HasValue)
    {
        cutoffDate = DateTimeOffset.UtcNow.AddDays(-daysLimit.Value);
    }

    // Fetch all messages (handle pagination)
    string? pageToken = null;
    int totalMessages = 0;

    var periodicTimer = new System.Timers.Timer(100)
    {
        AutoReset = true,
        Enabled = true
    };
    periodicTimer.Start();
    periodicTimer.Elapsed += (sender, e) =>
    {
        Console.Write($"\r{totalMessages}");
    };

    do
    {
        request.PageToken = pageToken;
        ListMessagesResponse response = await request.ExecuteAsync();

        if (response.Messages != null && response.Messages.Count > 0)
        {
            var allDates = new List<DateOnly?>();

            // Process messages in batches of MaxThreads
            for (int i = 0; i < response.Messages.Count; i += MaxThreads)
            {
                var batch = response.Messages.Skip(i).Take(MaxThreads).ToList();

                var tasks = batch.Select(async messageItem =>
                {
                    var msgRequest = service.Users.Messages.Get("me", messageItem.Id);
                    msgRequest.Format = UsersResource.MessagesResource.GetRequest.FormatEnum.Minimal; // Only get minimal data
                    var message = await msgRequest.ExecuteAsync();

                    var dateTime = ConvertUnixEpochToDateTime(message.InternalDate);

                    if (dateTime.HasValue)
                    {
                        // Check if message is within the days limit
                        if (cutoffDate.HasValue && dateTime.Value < cutoffDate.Value)
                        {
                            return (DateOnly?)null; // Skip messages older than the cutoff
                        }

                        var date = DateOnly.FromDateTime(dateTime.Value.ToLocalTime().DateTime);
                        return (DateOnly?)date;
                    }

                    return (DateOnly?)null;
                });

                var dates = await Task.WhenAll(tasks);
                allDates.AddRange(dates);
                totalMessages += dates.Count(d => d.HasValue);
            }

            foreach (var date in allDates)
            {
                if (date.HasValue)
                {
                    if (dateCountMap.ContainsKey(date.Value))
                    {
                        dateCountMap[date.Value]++;
                    }
                    else
                    {
                        dateCountMap[date.Value] = 1;
                    }
                }
            }
        }

        pageToken = response.NextPageToken;
    } while (pageToken != null);

    periodicTimer.Stop();
    periodicTimer.Dispose();

    // Display results
    Console.WriteLine("\rSpam emails by date:");
    if (dateCountMap.Count > 0)
    {
        foreach (var kvp in dateCountMap.OrderBy(x => x.Key))
        {
            Console.WriteLine($"{kvp.Key:ddd} {kvp.Key:yyyy-MM-dd} {kvp.Value}");
        }
        Console.WriteLine($"\nTotal: {totalMessages} spam email(s)");
    }
    else
    {
        Console.WriteLine("No spam messages found.");
    }
}
catch (FileNotFoundException)
{
    Console.WriteLine("Error: credentials.json not found. Please download it from Google Cloud Console and place it in the project directory.");
}
catch (Exception e)
{
    Console.WriteLine("An error occurred: " + e.Message);
}

static DateTimeOffset? ConvertUnixEpochToDateTime(long? epochMs)
{
    return epochMs.HasValue ? DateTimeOffset.FromUnixTimeMilliseconds(epochMs.Value) : null;
}
